//! The dependency solver, as upstream's (`Deps/Solver.hs`, the same search
//! in every supported release). nomi must pick what upstream picks, not
//! just some solution, so this is upstream's search: packages in name
//! order, each one's versions newest first, depth first, keeping the first
//! solution found. PubGrub, which would pick differently, only explains
//! why there is none ([`explain`]).

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::Compat;
use nomi_project::outline::AppOutline;
use nomi_project::{Constraint, Name, Outline, Version};

use crate::index::Known;

mod explain;

pub use explain::explain;

/// `Constraints`: what a release needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub elm: Constraint,
    pub deps: BTreeMap<Name, Constraint>,
}

impl Release {
    /// `constraintsDecoder`: a package's `elm.json` as the solver reads
    /// it, or `None` if it isn't one.
    pub fn decode(src: &[u8], compat: Compat) -> Option<Release> {
        match Outline::decode(src, compat) {
            Ok(Outline::Pkg(pkg)) => Some(Release {
                elm: pkg.elm,
                deps: pkg.deps,
            }),
            _ => None,
        }
    }
}

/// Where the solver finds releases: the index and their metadata.
pub trait Packages {
    type Error;

    /// The index's releases of `name`.
    fn versions(&self, name: &Name) -> Option<&Known>;

    /// `getConstraints`: what a release needs, or `None` when it can't be
    /// had, as when offline and not in the package cache, which sends the
    /// search elsewhere. An error ends the search.
    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, Self::Error>;

    /// Whether the releases come without the registry, which makes no
    /// solution a `NoOfflineSolution`.
    fn offline(&self) -> bool {
        false
    }
}

/// `Details`: a release in the solution, and what it depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    pub version: Version,
    pub deps: BTreeMap<Name, Constraint>,
}

/// `verify`: the releases that satisfy `constraints` and everything they
/// need, `None` if there are none.
pub fn verify<P: Packages>(
    packages: &mut P,
    compat: Compat,
    constraints: &BTreeMap<Name, Constraint>,
) -> Result<Option<BTreeMap<Name, Details>>, P::Error> {
    let mut search = Search::new(packages, compat);
    let Some(solution) = search.attempt(constraints.clone())? else {
        return Ok(None);
    };
    let details = solution
        .into_iter()
        .map(|(name, version)| {
            let deps = search.deps(&name, version).clone();
            (name, Details { version, deps })
        })
        .collect();
    Ok(Some(details))
}

/// `AppSolution`: an application's dependencies before and after adding
/// a package, or updating some.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSolution {
    pub old: BTreeMap<Name, Version>,
    pub new: BTreeMap<Name, Version>,
    pub app: AppOutline,
}

/// `addToApp`: the application with `pkg` among its direct dependencies,
/// or `None` if it can't be added. Each attempt frees more of the
/// application's dependencies: all held exactly, then only the direct
/// ones, then each direct one up to its next minor version, then up to
/// its next major one, then not at all.
pub fn add_to_app<P: Packages>(
    packages: &mut P,
    compat: Compat,
    pkg: &Name,
    app: &AppOutline,
) -> Result<Option<AppSolution>, P::Error> {
    let all_directs = union(&app.direct, &app.test_direct);
    let all_deps = union(&all_directs, &union(&app.indirect, &app.test_indirect));
    let relaxations: [Relaxation<'_>; 5] = [
        (&all_deps, &Constraint::exactly),
        (&all_directs, &Constraint::exactly),
        (&all_directs, &|v| Constraint::until_next_minor(v, compat)),
        (&all_directs, &|v| Constraint::until_next_major(v, compat)),
        (&all_directs, &|_| Constraint::anything(compat)),
    ];
    let mut search = Search::new(packages, compat);
    for (deps, to_constraint) in relaxations {
        let mut constraints: BTreeMap<Name, Constraint> = deps
            .iter()
            .map(|(name, &version)| (name.clone(), to_constraint(version)))
            .collect();
        constraints.insert(pkg.clone(), Constraint::anything(compat));
        if let Some(new) = search.attempt(constraints)? {
            let app = search.to_app(Some(pkg), app, &new);
            return Ok(Some(AppSolution {
                old: all_deps.clone(),
                new,
                app,
            }));
        }
    }
    Ok(None)
}

/// The application with the dependencies in `free` re-solved: each
/// direct one up to its next major version, each indirect one not at all.
/// The others are held exactly, then, if that fails, only the direct
/// ones. `None` if nothing works.
pub fn update_app<P: Packages>(
    packages: &mut P,
    compat: Compat,
    free: &BTreeSet<Name>,
    app: &AppOutline,
) -> Result<Option<AppSolution>, P::Error> {
    let all_directs = union(&app.direct, &app.test_direct);
    let all_deps = union(&all_directs, &union(&app.indirect, &app.test_indirect));
    let mut search = Search::new(packages, compat);
    for held in [&all_deps, &all_directs] {
        let mut constraints = BTreeMap::new();
        for (name, &version) in &all_deps {
            let constraint = if !free.contains(name) {
                held.contains_key(name)
                    .then(|| Constraint::exactly(version))
            } else {
                all_directs
                    .contains_key(name)
                    .then(|| Constraint::until_next_major(version, compat))
            };
            if let Some(constraint) = constraint {
                constraints.insert(name.clone(), constraint);
            }
        }
        if let Some(new) = search.attempt(constraints)? {
            let app = search.to_app(None, app, &new);
            return Ok(Some(AppSolution {
                old: all_deps.clone(),
                new,
                app,
            }));
        }
    }
    Ok(None)
}

/// Dependencies, and how freely to hold each.
type Relaxation<'a> = (
    &'a BTreeMap<Name, Version>,
    &'a dyn Fn(Version) -> Constraint,
);

/// `Map.union`: `a`'s entries, then `b`'s that `a` lacks.
fn union<V: Clone>(a: &BTreeMap<Name, V>, b: &BTreeMap<Name, V>) -> BTreeMap<Name, V> {
    let mut union = b.clone();
    union.extend(a.iter().map(|(k, v)| (k.clone(), v.clone())));
    union
}

/// The search and the metadata it has read, which outlives backtracking
/// as upstream's `State` does.
struct Search<'p, P> {
    packages: &'p mut P,
    compat: Compat,
    releases: BTreeMap<(Name, Version), Release>,
}

/// `Goals`: what is still to pick, and what has been.
#[derive(Clone)]
struct Goals {
    pending: BTreeMap<Name, Constraint>,
    solved: BTreeMap<Name, Version>,
}

impl<'p, P: Packages> Search<'p, P> {
    fn new(packages: &'p mut P, compat: Compat) -> Search<'p, P> {
        Search {
            packages,
            compat,
            releases: BTreeMap::new(),
        }
    }

    /// `try`.
    fn attempt(
        &mut self,
        constraints: BTreeMap<Name, Constraint>,
    ) -> Result<Option<BTreeMap<Name, Version>>, P::Error> {
        self.explore(Goals {
            pending: constraints,
            solved: BTreeMap::new(),
        })
    }

    /// `exploreGoals`: the first solution, taking the pending package
    /// first in name order and its versions newest first.
    fn explore(&mut self, mut goals: Goals) -> Result<Option<BTreeMap<Name, Version>>, P::Error> {
        let Some((name, constraint)) = goals.pending.pop_first() else {
            return Ok(Some(goals.solved));
        };
        // `getRelevantVersions`
        let versions: Vec<Version> = match self.packages.versions(&name) {
            Some(known) => known.all().filter(|&v| constraint.satisfies(v)).collect(),
            None => Vec::new(),
        };
        for version in versions {
            if let Some(next) = self.add_version(&goals, &name, version)?
                && let Some(solution) = self.explore(next)?
            {
                return Ok(Some(solution));
            }
        }
        Ok(None)
    }

    /// `addVersion`: the goals with `name` at `version`, if it can be.
    fn add_version(
        &mut self,
        goals: &Goals,
        name: &Name,
        version: Version,
    ) -> Result<Option<Goals>, P::Error> {
        let elm = Version::compiler(self.compat);
        let Some(release) = self.release(name, version)? else {
            return Ok(None);
        };
        // `C.goodElm`
        if !release.elm.satisfies(elm) {
            return Ok(None);
        }
        let mut pending = goals.pending.clone();
        for (dep, constraint) in &release.deps {
            // `addConstraint`
            if let Some(&solved) = goals.solved.get(dep) {
                if !constraint.satisfies(solved) {
                    return Ok(None);
                }
            } else if let Some(old) = pending.get_mut(dep) {
                match old.intersect(constraint) {
                    Some(merged) => *old = merged,
                    None => return Ok(None),
                }
            } else {
                pending.insert(dep.clone(), *constraint);
            }
        }
        let mut solved = goals.solved.clone();
        solved.insert(name.clone(), version);
        Ok(Some(Goals { pending, solved }))
    }

    /// `getConstraints`, remembering what it read.
    fn release(&mut self, name: &Name, version: Version) -> Result<Option<&Release>, P::Error> {
        let key = (name.clone(), version);
        if !self.releases.contains_key(&key) {
            match self.packages.release(name, version)? {
                Some(release) => {
                    self.releases.insert(key.clone(), release);
                }
                None => return Ok(None),
            }
        }
        Ok(self.releases.get(&key))
    }

    /// The dependencies of a release in a solution, which the search read.
    fn deps(&self, name: &Name, version: Version) -> &BTreeMap<Name, Constraint> {
        &self.releases[&(name.clone(), version)].deps
    }

    /// `toApp`: the application's dependencies, sorted as upstream sorts
    /// them. Direct ones stay direct, and `pkg` joins them; what those
    /// need is indirect; test dependencies stay so, unless something else
    /// needs them first.
    fn to_app(
        &self,
        pkg: Option<&Name>,
        app: &AppOutline,
        new: &BTreeMap<Name, Version>,
    ) -> AppOutline {
        let direct: BTreeMap<Name, Version> = new
            .iter()
            .filter(|(name, _)| Some(*name) == pkg || app.direct.contains_key(*name))
            .map(|(name, &version)| (name.clone(), version))
            .collect();
        let indirect: BTreeMap<Name, Version> = self
            .transitive(new, &direct)
            .into_iter()
            .filter(|name| !direct.contains_key(name))
            .map(|name| {
                let version = new[&name];
                (name, version)
            })
            .collect();
        let test_direct: BTreeMap<Name, Version> = new
            .iter()
            .filter(|(name, _)| Some(*name) != pkg && app.test_direct.contains_key(*name))
            .map(|(name, &version)| (name.clone(), version))
            .collect();
        let test_indirect = new
            .iter()
            .filter(|(name, _)| {
                !direct.contains_key(*name)
                    && !indirect.contains_key(*name)
                    && !test_direct.contains_key(*name)
            })
            .map(|(name, &version)| (name.clone(), version))
            .collect();
        AppOutline {
            elm: app.elm,
            source_dirs: app.source_dirs.clone(),
            direct,
            indirect,
            test_direct,
            test_indirect,
        }
    }

    /// `getTransitive`: the packages of `solution` that `roots` reach.
    fn transitive(
        &self,
        solution: &BTreeMap<Name, Version>,
        roots: &BTreeMap<Name, Version>,
    ) -> BTreeSet<Name> {
        let mut visited = BTreeSet::new();
        let mut unvisited: Vec<&Name> = roots.keys().collect();
        while let Some(name) = unvisited.pop() {
            if visited.insert(name.clone()) {
                let deps = self.deps(name, solution[name]);
                unvisited.extend(deps.keys().filter(|dep| solution.contains_key(*dep)));
            }
        }
        visited
    }
}
