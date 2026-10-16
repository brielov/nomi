//! How `update` plans bringing a project's dependencies up to date and
//! `outdated` finds newer releases, which upstream has no commands for.
//! An application's direct dependencies move up to their next major
//! version, which keeps their API, and what they need follows; a
//! package's ranges are already resolved to the newest releases they
//! allow whenever it builds, so there is nothing to write.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use nomi_base::Compat;
use nomi_project::outline::{AppOutline, PkgOutline};
use nomi_project::{Constraint, Name, Outline, Version};

use crate::index::Known;
use crate::install::{Change, changes};
use crate::solver::{self, Packages, ReleaseError, reachable, release_deps};

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

impl<E> From<ReleaseError<E>> for UpdateProblem<E> {
    fn from(e: ReleaseError<E>) -> UpdateProblem<E> {
        match e {
            ReleaseError::Unavailable(name, version) => UpdateProblem::Unavailable(name, version),
            ReleaseError::Solver(e) => UpdateProblem::Solver(e),
        }
    }
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
            let all = app.all_deps();
            let free = match pkg {
                None => all.keys().cloned().collect(),
                Some(pkg) if all.contains_key(pkg) => needing(&needs(packages, &all)?, pkg),
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

/// What updating several applications together does: the changes to
/// their dependencies, taken as one application's, and each one's new
/// dependencies, by its directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Together {
    pub changes: BTreeMap<Name, Change<Version>>,
    pub apps: Vec<(PathBuf, AppOutline)>,
}

/// What updating `apps`, each by its directory, would do: all their
/// dependencies, or only `pkg` and those that need it, solved as one
/// application's, so that they keep agreeing; then each application's
/// own, the indirect ones those it names need.
pub fn plan_together<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: Option<&Name>,
    apps: &[(PathBuf, &AppOutline)],
) -> Result<Together, UpdateProblem<P::Error>> {
    let all = union_of(apps.iter().map(|(_, app)| *app));
    let Plan::App { changes, outline } = plan(packages, compat, pkg, &Outline::App(all))? else {
        unreachable!("an application's plan is an application's")
    };
    let new = outline.all_deps();
    let mut each = Vec::with_capacity(apps.len());
    for (dir, app) in apps {
        each.push((dir.clone(), slice(packages, &new, app)?));
    }
    Ok(Together {
        changes,
        apps: each,
    })
}

/// The applications' dependencies as one application's: a name direct in
/// any is direct, else indirect in any, else a direct test one in any.
fn union_of<'a>(mut apps: impl Iterator<Item = &'a AppOutline>) -> AppOutline {
    let first = apps.next().expect("applications to update");
    let mut all = first.clone();
    for app in apps {
        all.direct
            .extend(app.direct.iter().map(|(n, &v)| (n.clone(), v)));
        all.indirect
            .extend(app.indirect.iter().map(|(n, &v)| (n.clone(), v)));
        all.test_direct
            .extend(app.test_direct.iter().map(|(n, &v)| (n.clone(), v)));
        all.test_indirect
            .extend(app.test_indirect.iter().map(|(n, &v)| (n.clone(), v)));
    }
    let direct = all.direct.clone();
    all.indirect.retain(|name, _| !direct.contains_key(name));
    let indirect = all.indirect.clone();
    all.test_direct
        .retain(|name, _| !direct.contains_key(name) && !indirect.contains_key(name));
    let test_direct = all.test_direct.clone();
    all.test_indirect.retain(|name, _| {
        !direct.contains_key(name)
            && !indirect.contains_key(name)
            && !test_direct.contains_key(name)
    });
    all
}

/// `app` with the releases of `new` its own dependencies name, and the
/// indirect ones they need.
fn slice<P: Packages>(
    packages: &mut P,
    new: &BTreeMap<Name, Version>,
    app: &AppOutline,
) -> Result<AppOutline, UpdateProblem<P::Error>> {
    let pick = |names: &BTreeMap<Name, Version>| -> BTreeMap<Name, Version> {
        (names.keys())
            .filter_map(|name| Some((name.clone(), *new.get(name)?)))
            .collect()
    };
    let direct = pick(&app.direct);
    let test_direct = pick(&app.test_direct);
    let mut deps = |name: &Name, version| release_deps(packages, name, version);
    let needed = reachable(new, &direct, &mut deps)?;
    let mut roots = direct.clone();
    roots.extend(test_direct.clone());
    let tests_need = reachable(new, &roots, &mut deps)?;
    let indirect: BTreeMap<Name, Version> = (needed.into_iter())
        .filter(|name| !direct.contains_key(name))
        .map(|name| (name.clone(), new[&name]))
        .collect();
    let test_indirect = (tests_need.into_iter())
        .filter(|name| !roots.contains_key(name) && !indirect.contains_key(name))
        .map(|name| (name.clone(), new[&name]))
        .collect();
    Ok(AppOutline {
        elm: app.elm,
        source_dirs: app.source_dirs.clone(),
        direct,
        indirect,
        test_direct,
        test_indirect,
    })
}

/// Which of a project's fields a dependency is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    /// `dependencies`, an application's `direct` ones.
    Normal,
    /// `test-dependencies`, an application's `direct` ones.
    Test,
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
    /// The newest release.
    pub latest: Version,
    pub kind: DepKind,
}

/// The application's direct dependencies with a release newer than the
/// version it holds.
pub fn outdated_app<P: Packages>(packages: &P, app: &AppOutline) -> Vec<Outdated<Version>> {
    outdated(
        packages,
        &app.direct,
        &app.test_direct,
        |current, known| known.newest > current,
        |current, known| {
            (known.all())
                .find(|v| v.major == current.major)
                .filter(|&v| v > current)
        },
    )
}

/// The package's dependencies with a release past the range it allows.
pub fn outdated_pkg<P: Packages>(packages: &P, pkg: &PkgOutline) -> Vec<Outdated<Constraint>> {
    outdated(
        packages,
        &pkg.deps,
        &pkg.test_deps,
        |current, known| !current.satisfies(known.newest),
        |current, known| known.all().find(|&v| current.satisfies(v)),
    )
}

/// The dependencies, then the test ones, that the index knows releases
/// `newer` than, with the release each may move to.
fn outdated<T: Copy>(
    packages: &impl Packages,
    deps: &BTreeMap<Name, T>,
    test_deps: &BTreeMap<Name, T>,
    newer: impl Fn(T, &Known) -> bool,
    compatible: impl Fn(T, &Known) -> Option<Version>,
) -> Vec<Outdated<T>> {
    let deps = (deps.iter().map(|dep| (dep, DepKind::Normal)))
        .chain(test_deps.iter().map(|dep| (dep, DepKind::Test)));
    deps.filter_map(|((name, &current), kind)| {
        let known = packages.versions(name)?;
        newer(current, known).then(|| Outdated {
            name: name.clone(),
            current,
            compatible: compatible(current, known),
            latest: known.newest,
            kind,
        })
    })
    .collect()
}

/// What each package of a solution needs.
type Needs<'s> = BTreeMap<&'s Name, BTreeMap<Name, Constraint>>;

/// What each release of `solution` needs.
fn needs<'s, P: Packages>(
    packages: &mut P,
    solution: &'s BTreeMap<Name, Version>,
) -> Result<Needs<'s>, UpdateProblem<P::Error>> {
    let mut needs = BTreeMap::new();
    for (name, &version) in solution {
        needs.insert(name, release_deps(packages, name, version)?);
    }
    Ok(needs)
}

/// `pkg` and the packages that need it, directly or not, by what each
/// needs.
fn needing(needs: &Needs<'_>, pkg: &Name) -> BTreeSet<Name> {
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
            return needing;
        }
        needing.extend(more);
    }
}

#[cfg(test)]
mod tests {
    use nomi_project::outline::SrcDir;

    use super::*;
    use crate::fixture::{Fixture, name, version as v};

    const COMPAT: Compat = Compat::LATEST;

    /// elm/http has 2.0.0 and 1.1.0 past 1.0.0, elm/json nothing past
    /// 1.1.3, and elm/test 2.1.0 past 2.0.0.
    fn index() -> Fixture {
        let known = |all: &[&str]| Known {
            newest: v(all[0]),
            previous: all[1..].iter().map(|s| v(s)).collect(),
        };
        let index = BTreeMap::from([
            (name("elm/http"), known(&["2.0.0", "1.1.0", "1.0.0"])),
            (name("elm/json"), known(&["1.1.3", "1.0.0"])),
            (name("elm/test"), known(&["2.1.0", "2.0.0"])),
        ]);
        Fixture {
            index,
            ..Fixture::default()
        }
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
                    kind: DepKind::Normal,
                },
                Outdated {
                    name: name("elm/test"),
                    current: v("2.0.0"),
                    compatible: Some(v("2.1.0")),
                    latest: v("2.1.0"),
                    kind: DepKind::Test,
                },
            ]
        );
    }

    #[test]
    fn what_needs_a_package_directly_or_not_is_updated_with_it() {
        let needing_of = |deps: &[&str]| -> BTreeMap<Name, Constraint> {
            (deps.iter())
                .map(|dep| (name(dep), Constraint::anything(COMPAT)))
                .collect()
        };
        let [core, json, http, test] = ["elm/core", "elm/json", "elm/http", "elm/test"].map(name);
        let needs = BTreeMap::from([
            (&core, needing_of(&[])),
            (&json, needing_of(&["elm/core"])),
            (&http, needing_of(&["elm/json"])),
            (&test, needing_of(&[])),
        ]);
        assert_eq!(
            needing(&needs, &core),
            BTreeSet::from([core.clone(), json.clone(), http.clone()])
        );
        assert_eq!(needing(&needs, &test), BTreeSet::from([test.clone()]));
    }

    #[test]
    fn a_package_hears_of_releases_past_its_ranges() {
        let src = br#"{
            "type": "package", "name": "a/b", "summary": "", "license": "BSD-3-Clause",
            "version": "1.0.0", "exposed-modules": [], "elm-version": "0.19.0 <= v < 0.20.0",
            "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0", "elm/http": "1.0.0 <= v < 2.0.0",
                "elm/json": "1.0.0 <= v < 2.0.0"},
            "test-dependencies": {}
        }"#;
        let Ok(Outline::Pkg(pkg)) = Outline::read(src, COMPAT) else {
            panic!("not a package")
        };
        assert_eq!(
            outdated_pkg(&index(), &pkg),
            [Outdated {
                name: name("elm/http"),
                current: Constraint::until_next_major(v("1.0.0"), COMPAT),
                compatible: Some(v("1.1.0")),
                latest: v("2.0.0"),
                kind: DepKind::Normal,
            }]
        );
    }
}
