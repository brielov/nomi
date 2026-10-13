//! How `rm` plans taking a package out of a project, which upstream has
//! no command for. An application keeps the versions it has: what the
//! remaining direct dependencies need stays, at the same versions, and
//! what nothing needs any more goes, so the result is what `install`
//! would have written for the remaining dependencies.

use std::collections::BTreeMap;

use nomi_project::outline::{AppOutline, PkgOutline};
use nomi_project::{Constraint, Name, Outline, Version};

use crate::install::{Change, changes};
use crate::solver::{self, Packages, ReleaseError, release_deps};

/// What removing the package does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// An application's new dependencies.
    App {
        changes: BTreeMap<Name, Change<Version>>,
        outline: AppOutline,
    },
    /// A package's new dependencies.
    Pkg {
        changes: BTreeMap<Name, Change<Constraint>>,
        outline: PkgOutline,
    },
}

/// Why the package can't be removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveProblem<E> {
    /// The project doesn't depend on it.
    NotADependency,
    /// The application depends on it only through the packages named,
    /// so it goes when they do.
    Indirect(Vec<Name>),
    /// A dependency's `elm.json` can't be had, as when offline and not
    /// downloaded, so what it needs is unknown.
    Unavailable(Name, Version),
    /// Reading a dependency's `elm.json` failed.
    Solver(E),
}

impl<E> From<ReleaseError<E>> for RemoveProblem<E> {
    fn from(e: ReleaseError<E>) -> RemoveProblem<E> {
        match e {
            ReleaseError::Unavailable(name, version) => RemoveProblem::Unavailable(name, version),
            ReleaseError::Solver(e) => RemoveProblem::Solver(e),
        }
    }
}

/// What removing `pkg` from the project would do.
pub fn plan<P: Packages>(
    packages: &mut P,
    pkg: &Name,
    outline: &Outline,
) -> Result<Plan, RemoveProblem<P::Error>> {
    match outline {
        Outline::App(app) => plan_app(packages, pkg, app),
        Outline::Pkg(outline) => plan_pkg(pkg, outline),
    }
}

fn plan_app<P: Packages>(
    packages: &mut P,
    pkg: &Name,
    app: &AppOutline,
) -> Result<Plan, RemoveProblem<P::Error>> {
    let old = app.all_deps();
    let mut direct = app.direct.clone();
    let mut test_direct = app.test_direct.clone();
    if direct.remove(pkg).is_none() && test_direct.remove(pkg).is_none() {
        return Err(if old.contains_key(pkg) {
            RemoveProblem::Indirect(dependents(packages, pkg, &old)?)
        } else {
            RemoveProblem::NotADependency
        });
    }
    let indirect = reached(packages, &old, &direct, &[&direct])?;
    let test_indirect = reached(
        packages,
        &old,
        &test_direct,
        &[&direct, &indirect, &test_direct],
    )?;
    let outline = AppOutline {
        elm: app.elm,
        source_dirs: app.source_dirs.clone(),
        direct,
        indirect,
        test_direct,
        test_indirect,
    };
    Ok(Plan::App {
        changes: changes(&old, &outline.all_deps()),
        outline,
    })
}

fn plan_pkg<E>(pkg: &Name, outline: &PkgOutline) -> Result<Plan, RemoveProblem<E>> {
    let mut outline = outline.clone();
    let constraint = match outline.deps.remove(pkg) {
        Some(constraint) => constraint,
        None => outline
            .test_deps
            .remove(pkg)
            .ok_or(RemoveProblem::NotADependency)?,
    };
    Ok(Plan::Pkg {
        changes: BTreeMap::from([(pkg.clone(), Change::Remove(constraint))]),
        outline,
    })
}

/// The packages of `solution` that `roots` need, directly or not, at
/// their versions there, leaving out those in `known`.
fn reached<P: Packages>(
    packages: &mut P,
    solution: &BTreeMap<Name, Version>,
    roots: &BTreeMap<Name, Version>,
    known: &[&BTreeMap<Name, Version>],
) -> Result<BTreeMap<Name, Version>, RemoveProblem<P::Error>> {
    let reached = solver::reachable(solution, roots, |name, version| {
        release_deps(packages, name, version)
    })?;
    Ok(reached
        .into_iter()
        .filter(|name| !known.iter().any(|deps| deps.contains_key(name)))
        .map(|name| {
            let version = solution[&name];
            (name, version)
        })
        .collect())
}

/// The packages of `solution` that need `pkg`.
fn dependents<P: Packages>(
    packages: &mut P,
    pkg: &Name,
    solution: &BTreeMap<Name, Version>,
) -> Result<Vec<Name>, RemoveProblem<P::Error>> {
    let mut dependents = Vec::new();
    for (name, &version) in solution {
        if release_deps(packages, name, version)?.contains_key(pkg) {
            dependents.push(name.clone());
        }
    }
    Ok(dependents)
}

#[cfg(test)]
mod tests {
    use nomi_base::Compat;

    use super::*;
    use crate::fixture::{Fixture, name};

    const COMPAT: Compat = Compat::LATEST;

    fn deps(names: &[&str]) -> BTreeMap<Name, Version> {
        names
            .iter()
            .map(|n| (name(n), Version::new(1, 0, 0)))
            .collect()
    }

    /// http needs json and bytes, test needs random, and json is also a
    /// test dependency's.
    fn fixture() -> (Fixture, AppOutline) {
        let needs = BTreeMap::from([
            (name("elm/core"), vec![]),
            (name("elm/http"), vec![name("elm/json"), name("elm/bytes")]),
            (name("elm/json"), vec![]),
            (name("elm/bytes"), vec![]),
            (name("elm/test"), vec![name("elm/random"), name("elm/json")]),
            (name("elm/random"), vec![]),
        ]);
        let releases = Fixture {
            needs,
            ..Fixture::default()
        };
        let app = AppOutline {
            elm: Version::compiler(COMPAT),
            source_dirs: vec![nomi_project::outline::SrcDir::new("src")],
            direct: deps(&["elm/core", "elm/http"]),
            indirect: deps(&["elm/bytes", "elm/json"]),
            test_direct: deps(&["elm/test"]),
            test_indirect: deps(&["elm/random"]),
        };
        (releases, app)
    }

    #[test]
    fn what_only_the_package_needed_goes() {
        let (mut releases, app) = fixture();
        let Ok(Plan::App { changes, outline }) =
            plan(&mut releases, &name("elm/http"), &Outline::App(app))
        else {
            panic!()
        };
        assert_eq!(outline.direct, deps(&["elm/core"]));
        assert_eq!(outline.indirect, deps(&[]));
        assert_eq!(outline.test_direct, deps(&["elm/test"]));
        assert_eq!(outline.test_indirect, deps(&["elm/json", "elm/random"]));
        let removed: Vec<_> = changes.keys().map(Name::to_string).collect();
        assert_eq!(removed, ["elm/bytes", "elm/http"]);
    }

    #[test]
    fn test_dependencies_go_the_same_way() {
        let (mut releases, app) = fixture();
        let Ok(Plan::App { outline, .. }) =
            plan(&mut releases, &name("elm/test"), &Outline::App(app))
        else {
            panic!()
        };
        assert_eq!(outline.indirect, deps(&["elm/bytes", "elm/json"]));
        assert_eq!(outline.test_direct, deps(&[]));
        assert_eq!(outline.test_indirect, deps(&[]));
    }

    #[test]
    fn indirect_and_unknown_dependencies_stay() {
        let (mut releases, app) = fixture();
        let app = Outline::App(app);
        assert_eq!(
            plan(&mut releases, &name("elm/json"), &app),
            Err(RemoveProblem::Indirect(vec![
                name("elm/http"),
                name("elm/test")
            ]))
        );
        assert_eq!(
            plan(&mut releases, &name("elm/svg"), &app),
            Err(RemoveProblem::NotADependency)
        );
        releases.needs.remove(&name("elm/http"));
        assert_eq!(
            plan(&mut releases, &name("elm/core"), &app),
            Err(RemoveProblem::Unavailable(
                name("elm/http"),
                Version::new(1, 0, 0)
            ))
        );
    }

    #[test]
    fn a_package_loses_the_constraint() {
        let src = br#"{
            "type": "package", "name": "a/b", "summary": "", "license": "BSD-3-Clause",
            "version": "1.0.0", "exposed-modules": [], "elm-version": "0.19.0 <= v < 0.20.0",
            "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"},
            "test-dependencies": {"elm/test": "2.0.0 <= v < 3.0.0"}
        }"#;
        let outline = Outline::read(src, COMPAT).unwrap();
        let (mut releases, _) = fixture();
        let Ok(Plan::Pkg { outline, .. }) = plan(&mut releases, &name("elm/test"), &outline) else {
            panic!()
        };
        assert!(outline.test_deps.is_empty());
        assert_eq!(outline.deps.len(), 1);
        let pkg = Outline::Pkg(outline);
        assert_eq!(
            plan(&mut releases, &name("elm/json"), &pkg),
            Err(RemoveProblem::NotADependency)
        );
    }
}
