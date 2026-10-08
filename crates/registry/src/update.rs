//! How `update` plans bringing a project's dependencies up to date and
//! `outdated` finds newer releases, which upstream has no commands for.
//! An application's direct dependencies move up to their next major
//! version, which keeps their API, and what they need follows; a
//! package's ranges are already resolved to the newest releases they
//! allow whenever it builds, so there is nothing to write.

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;
use nomi_project::outline::AppOutline;
use nomi_project::{Constraint, Name, Outline, Version};

use crate::install::{Change, changes};
use crate::solver::{self, Packages};

/// What updating does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// An application's new dependencies; no changes when they are up to
    /// date.
    App {
        changes: BTreeMap<Name, Change<Version>>,
        outline: AppOutline,
    },
    /// A package, whose ranges stay.
    Pkg,
}

/// Why the dependencies can't be updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateProblem<E> {
    /// The project doesn't depend on the package named.
    NotADependency,
    /// A dependency's `elm.json` can't be had, so what needs the package
    /// named is unknown.
    Unavailable(Name, Version),
    /// No releases satisfy the application any more, as when one it
    /// holds was withdrawn.
    NoSolution,
    /// Reading a release failed.
    Solver(E),
}

/// What updating the project's dependencies would do: all of them, or
/// only `pkg` and those that need it.
pub fn plan<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: Option<&Name>,
    outline: &Outline,
) -> Result<Plan, UpdateProblem<P::Error>> {
    match outline {
        Outline::Pkg(outline) => match pkg {
            Some(pkg)
                if !outline.deps.contains_key(pkg) && !outline.test_deps.contains_key(pkg) =>
            {
                Err(UpdateProblem::NotADependency)
            }
            _ => Ok(Plan::Pkg),
        },
        Outline::App(app) => {
            let all = all_deps(app);
            let free = match pkg {
                None => all.keys().cloned().collect(),
                Some(pkg) if all.contains_key(pkg) => needing(packages, pkg, &all)?,
                Some(_) => return Err(UpdateProblem::NotADependency),
            };
            let solution = solver::update_app(packages, compat, &free, app)
                .map_err(UpdateProblem::Solver)?
                .ok_or(UpdateProblem::NoSolution)?;
            Ok(Plan::App {
                changes: changes(&solution.old, &solution.new),
                outline: solution.app,
            })
        }
    }
}

/// A dependency with newer releases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outdated<T> {
    pub name: Name,
    /// The version an application holds, or the range a package allows.
    pub current: T,
    /// The newest release `update` may move to: an application's newest
    /// of the same major version, a package's newest in range.
    pub compatible: Option<Version>,
    pub latest: Version,
    pub test: bool,
}

/// The application's direct dependencies with a release newer than the
/// version it holds.
pub fn outdated_app<P: Packages>(packages: &P, app: &AppOutline) -> Vec<Outdated<Version>> {
    let deps = app
        .direct
        .iter()
        .map(|dep| (dep, false))
        .chain(app.test_direct.iter().map(|dep| (dep, true)));
    deps.filter_map(|((name, &current), test)| {
        let known = packages.versions(name)?;
        (known.newest > current).then(|| Outdated {
            name: name.clone(),
            current,
            compatible: known
                .all()
                .find(|v| v.major == current.major)
                .filter(|&v| v > current),
            latest: known.newest,
            test,
        })
    })
    .collect()
}

/// The package's dependencies with a release past the range it allows.
pub fn outdated_pkg<P: Packages>(
    packages: &P,
    deps: &BTreeMap<Name, Constraint>,
    test_deps: &BTreeMap<Name, Constraint>,
) -> Vec<Outdated<Constraint>> {
    let deps = deps
        .iter()
        .map(|dep| (dep, false))
        .chain(test_deps.iter().map(|dep| (dep, true)));
    deps.filter_map(|((name, &current), test)| {
        let known = packages.versions(name)?;
        (!current.satisfies(known.newest)).then(|| Outdated {
            name: name.clone(),
            current,
            compatible: known.all().find(|&v| current.satisfies(v)),
            latest: known.newest,
            test,
        })
    })
    .collect()
}

/// Every dependency of the application, at its version.
fn all_deps(app: &AppOutline) -> BTreeMap<Name, Version> {
    let mut all = app.test_indirect.clone();
    for deps in [&app.test_direct, &app.indirect, &app.direct] {
        all.extend(deps.iter().map(|(name, &version)| (name.clone(), version)));
    }
    all
}

/// `pkg` and the packages of `solution` that need it, directly or not.
fn needing<P: Packages>(
    packages: &mut P,
    pkg: &Name,
    solution: &BTreeMap<Name, Version>,
) -> Result<BTreeSet<Name>, UpdateProblem<P::Error>> {
    let mut needs = BTreeMap::new();
    for (name, &version) in solution {
        let deps = match packages.release(name, version) {
            Ok(Some(release)) => release.deps,
            Ok(None) => return Err(UpdateProblem::Unavailable(name.clone(), version)),
            Err(e) => return Err(UpdateProblem::Solver(e)),
        };
        needs.insert(name, deps);
    }
    let mut needing = BTreeSet::from([pkg.clone()]);
    loop {
        let more: Vec<Name> = needs
            .iter()
            .filter(|(name, deps)| {
                !needing.contains(**name) && deps.keys().any(|dep| needing.contains(dep))
            })
            .map(|(name, _)| (*name).clone())
            .collect();
        if more.is_empty() {
            return Ok(needing);
        }
        needing.extend(more);
    }
}

#[cfg(test)]
mod tests {
    use nomi_project::json::Cursor;
    use nomi_project::outline::SrcDir;

    use super::*;
    use crate::index::Known;
    use crate::solver::Release;

    const COMPAT: Compat = Compat::LATEST;

    struct Index(BTreeMap<Name, Known>);

    impl Packages for Index {
        type Error = ();

        fn versions(&self, name: &Name) -> Option<&Known> {
            self.0.get(name)
        }

        fn release(&mut self, _: &Name, _: Version) -> Result<Option<Release>, ()> {
            Ok(None)
        }
    }

    fn name(s: &str) -> Name {
        Name::parse(s.as_bytes(), Cursor::start(COMPAT), COMPAT).unwrap()
    }

    fn v(s: &str) -> Version {
        Version::parse(s.as_bytes(), Cursor::start(COMPAT), COMPAT).unwrap()
    }

    /// elm/http has 2.0.0 and 1.1.0 past 1.0.0, elm/json nothing past
    /// 1.1.3, and elm/test 2.1.0 past 2.0.0.
    fn index() -> Index {
        let known = |all: &[&str]| Known {
            newest: v(all[0]),
            previous: all[1..].iter().map(|s| v(s)).collect(),
        };
        Index(BTreeMap::from([
            (name("elm/http"), known(&["2.0.0", "1.1.0", "1.0.0"])),
            (name("elm/json"), known(&["1.1.3", "1.0.0"])),
            (name("elm/test"), known(&["2.1.0", "2.0.0"])),
        ]))
    }

    #[test]
    fn an_application_hears_of_newer_releases() {
        let app = AppOutline {
            elm: Version::compiler(COMPAT),
            source_dirs: vec![SrcDir::new("src")],
            direct: BTreeMap::from([
                (name("elm/http"), v("1.0.0")),
                (name("elm/json"), v("1.1.3")),
            ]),
            indirect: BTreeMap::new(),
            test_direct: BTreeMap::from([(name("elm/test"), v("2.0.0"))]),
            test_indirect: BTreeMap::new(),
        };
        assert_eq!(
            outdated_app(&index(), &app),
            [
                Outdated {
                    name: name("elm/http"),
                    current: v("1.0.0"),
                    compatible: Some(v("1.1.0")),
                    latest: v("2.0.0"),
                    test: false,
                },
                Outdated {
                    name: name("elm/test"),
                    current: v("2.0.0"),
                    compatible: Some(v("2.1.0")),
                    latest: v("2.1.0"),
                    test: true,
                },
            ]
        );
    }

    #[test]
    fn a_package_hears_of_releases_past_its_ranges() {
        let deps = BTreeMap::from([
            (
                name("elm/http"),
                Constraint::until_next_major(v("1.0.0"), COMPAT),
            ),
            (
                name("elm/json"),
                Constraint::until_next_major(v("1.0.0"), COMPAT),
            ),
        ]);
        assert_eq!(
            outdated_pkg(&index(), &deps, &BTreeMap::new()),
            [Outdated {
                name: name("elm/http"),
                current: Constraint::until_next_major(v("1.0.0"), COMPAT),
                compatible: Some(v("1.1.0")),
                latest: v("2.0.0"),
                test: false,
            }]
        );
    }
}
