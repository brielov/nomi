//! How a build checks a project's dependencies before downloading them
//! (`Elm/Details.hs`, `verifyPkg` and `verifyApp`).

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_project::{Constraint, Name, Outline, Version};

use crate::bundle;
use crate::solver::{self, Details, Packages};

/// Why a project's dependencies don't check out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetailsProblem<E> {
    /// `DetailsBadElmInAppOutline`: the application is for another elm.
    AppElm(Version),
    /// `DetailsBadElmInPkg`: the package excludes this elm.
    PkgElm(Constraint),
    /// `DetailsHandEditedDependencies`: the dependencies contradict each
    /// other, or an application's aren't exactly its solution.
    HandEdited,
    /// No releases satisfy the dependencies.
    NoSolution,
    /// None satisfy them that can be had without the registry, which
    /// couldn't be reached.
    NoOfflineSolution,
    /// Reading a release failed.
    Solver(E),
    /// The project asks for releases of one of nomi's own packages, which
    /// come with nomi alone, and the release nomi bundles isn't one: for
    /// an application, `wanted` is exactly the version it names.
    Unbundled {
        name: Name,
        wanted: Constraint,
        bundled: Version,
    },
}

/// The releases a project builds with, each with what it depends on.
pub fn verify<P: Packages>(
    packages: &mut P,
    compat: Compat,
    outline: &Outline,
) -> Result<BTreeMap<Name, Details>, DetailsProblem<P::Error>> {
    match outline {
        Outline::Pkg(pkg) => {
            if !pkg.elm.satisfies(Version::compiler(compat)) {
                return Err(DetailsProblem::PkgElm(pkg.elm));
            }
            let constraints = union_agreeing(&pkg.deps, &pkg.test_deps, |_, _| false)
                .ok_or(DetailsProblem::HandEdited)?;
            bundled(packages, &constraints)?;
            solve(packages, compat, &constraints)
        }
        Outline::App(app) => {
            if app.elm != Version::compiler(compat) {
                return Err(DetailsProblem::AppElm(app.elm));
            }
            // `checkAppDeps`
            let stated = union_agreeing(&app.indirect, &app.test_direct, |a, b| a == b)
                .and_then(|x| {
                    let y = union_agreeing(&app.direct, &app.test_indirect, |_, _| false)?;
                    union_agreeing(&x, &y, |_, _| false)
                })
                .ok_or(DetailsProblem::HandEdited)?;
            let constraints = stated
                .iter()
                .map(|(name, &version)| (name.clone(), Constraint::exactly(version)))
                .collect();
            bundled(packages, &constraints)?;
            let solution = solve(packages, compat, &constraints)?;
            if solution.len() != stated.len() {
                return Err(DetailsProblem::HandEdited);
            }
            Ok(solution)
        }
    }
}

/// That each of nomi's own packages among `constraints` is asked for at
/// a release nomi bundles, which are the only releases `packages` has.
fn bundled<P: Packages>(
    packages: &P,
    constraints: &BTreeMap<Name, Constraint>,
) -> Result<(), DetailsProblem<P::Error>> {
    for (name, wanted) in constraints {
        let Some(known) = packages.versions(name).filter(|_| bundle::is_bundled(name)) else {
            continue;
        };
        let mut releases = std::iter::once(&known.newest).chain(&known.previous);
        if !releases.any(|&version| wanted.satisfies(version)) {
            return Err(DetailsProblem::Unbundled {
                name: name.clone(),
                wanted: *wanted,
                bundled: known.newest,
            });
        }
    }
    Ok(())
}

/// The releases that satisfy `constraints`, or, where there are none,
/// why: offline or not.
pub(crate) fn solve<P: Packages>(
    packages: &mut P,
    compat: Compat,
    constraints: &BTreeMap<Name, Constraint>,
) -> Result<BTreeMap<Name, Details>, DetailsProblem<P::Error>> {
    solver::verify(packages, compat, constraints)
        .map_err(DetailsProblem::Solver)?
        .ok_or_else(|| no_solution(packages))
}

/// That nothing satisfies the constraints, offline or not.
pub(crate) fn no_solution<P: Packages>(packages: &P) -> DetailsProblem<P::Error> {
    if packages.offline() {
        DetailsProblem::NoOfflineSolution
    } else {
        DetailsProblem::NoSolution
    }
}

/// `union`: both maps, if every name in both is `allowed` there.
fn union_agreeing<V: Clone>(
    a: &BTreeMap<Name, V>,
    b: &BTreeMap<Name, V>,
    allowed: impl Fn(&V, &V) -> bool,
) -> Option<BTreeMap<Name, V>> {
    let mut union = a.clone();
    for (name, v) in b {
        match a.get(name) {
            Some(old) if !allowed(old, v) => return None,
            Some(_) => {}
            None => {
                union.insert(name.clone(), v.clone());
            }
        }
    }
    Some(union)
}
