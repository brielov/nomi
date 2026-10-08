//! How `install` plans adding a package to a project (`Install.hs`,
//! `makeAppPlan` and `makePkgPlan`), short of asking and writing.

use std::collections::BTreeMap;

use nomi_base::Compat;
use nomi_project::outline::{AppOutline, PkgOutline};
use nomi_project::{Constraint, Name, Outline, Version};

use crate::solver::{self, Packages};

/// `Changes`: what adding the package does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    AlreadyInstalled,
    /// The package was an indirect dependency, and becomes a direct one.
    PromoteIndirect(Outline),
    /// The package was a test dependency, and becomes a direct one.
    PromoteTest(Outline),
    /// An application's new dependencies.
    App {
        changes: BTreeMap<Name, Change<Version>>,
        outline: AppOutline,
    },
    /// A package's new dependencies, and the releases that satisfy them.
    Pkg {
        changes: BTreeMap<Name, Change<Constraint>>,
        outline: PkgOutline,
        solution: BTreeMap<Name, Version>,
    },
}

/// `Change`: how one dependency changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<T> {
    Insert(T),
    Change(T, T),
    Remove(T),
}

/// Why the package can't be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallProblem<E> {
    /// The index has no such package.
    UnknownPackage,
    /// No releases satisfy the project with the package added.
    NoSolution,
    /// Reading a release failed.
    Solver(E),
}

/// What adding `pkg` to the project would do.
pub fn plan<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: &Name,
    outline: &Outline,
) -> Result<Plan, InstallProblem<P::Error>> {
    match outline {
        Outline::App(app) => plan_app(packages, compat, pkg, app),
        Outline::Pkg(outline) => plan_pkg(packages, compat, pkg, outline),
    }
}

/// `makeAppPlan`.
fn plan_app<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: &Name,
    app: &AppOutline,
) -> Result<Plan, InstallProblem<P::Error>> {
    if app.direct.contains_key(pkg) {
        return Ok(Plan::AlreadyInstalled);
    }
    let promote = |from: fn(&mut AppOutline) -> &mut BTreeMap<Name, Version>| {
        let mut app = app.clone();
        let version = from(&mut app).remove(pkg)?;
        app.direct.insert(pkg.clone(), version);
        Some(Outline::App(app))
    };
    if let Some(outline) = promote(|app| &mut app.indirect) {
        return Ok(Plan::PromoteIndirect(outline));
    }
    if let Some(outline) =
        promote(|app| &mut app.test_direct).or_else(|| promote(|app| &mut app.test_indirect))
    {
        return Ok(Plan::PromoteTest(outline));
    }
    if packages.versions(pkg).is_none() {
        return Err(InstallProblem::UnknownPackage);
    }
    let solution = solver::add_to_app(packages, compat, pkg, app)
        .map_err(InstallProblem::Solver)?
        .ok_or(InstallProblem::NoSolution)?;
    Ok(Plan::App {
        changes: changes(&solution.old, &solution.new),
        outline: solution.app,
    })
}

/// `makePkgPlan`: the package joins the dependencies up to its next major
/// version, and dependencies it moves keep their places.
fn plan_pkg<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: &Name,
    outline: &PkgOutline,
) -> Result<Plan, InstallProblem<P::Error>> {
    if outline.deps.contains_key(pkg) {
        return Ok(Plan::AlreadyInstalled);
    }
    if let Some(&constraint) = outline.test_deps.get(pkg) {
        let mut outline = outline.clone();
        outline.test_deps.remove(pkg);
        outline.deps.insert(pkg.clone(), constraint);
        return Ok(Plan::PromoteTest(Outline::Pkg(outline)));
    }
    if packages.versions(pkg).is_none() {
        return Err(InstallProblem::UnknownPackage);
    }
    let mut old = outline.test_deps.clone();
    old.extend(outline.deps.iter().map(|(n, &c)| (n.clone(), c)));
    let mut constraints = old.clone();
    constraints.insert(pkg.clone(), Constraint::anything(compat));
    let solution = solver::verify(packages, compat, &constraints)
        .map_err(InstallProblem::Solver)?
        .ok_or(InstallProblem::NoSolution)?;
    let version = solution[pkg].version;
    let mut new = old.clone();
    new.insert(pkg.clone(), Constraint::until_next_major(version, compat));
    let changes = changes(&old, &new);
    let news = changes.iter().filter_map(|(name, change)| match change {
        Change::Insert(c) | Change::Change(_, c) => Some((name, *c)),
        Change::Remove(_) => None,
    });
    let mut outline = outline.clone();
    // `addNews`: new constraints replace old ones, and only the package
    // itself joins
    for (name, constraint) in news {
        if name == pkg || outline.deps.contains_key(name) {
            outline.deps.insert(name.clone(), constraint);
        }
        if let Some(c) = outline.test_deps.get_mut(name) {
            *c = constraint;
        }
    }
    Ok(Plan::Pkg {
        changes,
        outline,
        solution: solution
            .into_iter()
            .map(|(name, details)| (name, details.version))
            .collect(),
    })
}

/// `detectChanges`.
fn changes<T: Copy + PartialEq>(
    old: &BTreeMap<Name, T>,
    new: &BTreeMap<Name, T>,
) -> BTreeMap<Name, Change<T>> {
    let removed = old
        .iter()
        .filter(|(name, _)| !new.contains_key(*name))
        .map(|(name, &v)| (name.clone(), Change::Remove(v)));
    let rest = new.iter().filter_map(|(name, &v)| match old.get(name) {
        None => Some((name.clone(), Change::Insert(v))),
        Some(&o) if o != v => Some((name.clone(), Change::Change(o, v))),
        Some(_) => None,
    });
    removed.chain(rest).collect()
}
