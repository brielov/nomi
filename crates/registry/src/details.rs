//! How a build checks a project's dependencies before downloading them
//! (`Elm/Details.hs`, `verifyPkg` and `verifyApp`).

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_project::{Constraint, Name, Outline, Version};

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
    /// Reading a release failed.
    Solver(E),
}

/// The releases a project builds with, each with what it depends on.
pub fn verify<P: Packages>(
    packages: &mut P,
    compat: Compat,
    outline: &Outline,
) -> Result<BTreeMap<Name, Details>, DetailsProblem<P::Error>> {
    let solve = |packages: &mut P, constraints| {
        solver::verify(packages, compat, &constraints)
            .map_err(DetailsProblem::Solver)?
            .ok_or(DetailsProblem::NoSolution)
    };
    match outline {
        Outline::Pkg(pkg) => {
            if !pkg.elm.satisfies(Version::compiler(compat)) {
                return Err(DetailsProblem::PkgElm(pkg.elm));
            }
            let constraints =
                union(&pkg.deps, &pkg.test_deps, |_, _| false).ok_or(DetailsProblem::HandEdited)?;
            solve(packages, constraints)
        }
        Outline::App(app) => {
            if app.elm != Version::compiler(compat) {
                return Err(DetailsProblem::AppElm(app.elm));
            }
            // `checkAppDeps`
            let stated = union(&app.indirect, &app.test_direct, |a, b| a == b)
                .and_then(|x| {
                    let y = union(&app.direct, &app.test_indirect, |_, _| false)?;
                    union(&x, &y, |_, _| false)
                })
                .ok_or(DetailsProblem::HandEdited)?;
            let constraints = stated
                .iter()
                .map(|(name, &version)| (name.clone(), Constraint::exactly(version)))
                .collect();
            let solution = solve(packages, constraints)?;
            if solution.len() != stated.len() {
                return Err(DetailsProblem::HandEdited);
            }
            Ok(solution)
        }
    }
}

/// `union`: both maps, if every name in both is `allowed` there.
fn union<V: Clone>(
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
