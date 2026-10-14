//! The dependency solver, as upstream's (`Deps/Solver.hs`, the same search
//! in every supported release). nomi must pick what upstream picks, not
//! just some solution, so this is upstream's search: packages in name
//! order, each one's versions newest first, depth first, keeping the first
//! solution found. `PubGrub`, which would pick differently, only explains
//! why there is none ([`explain`]).

use std::borrow::Borrow;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

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
            Ok(Outline::App(_)) | Err(_) => None,
        }
    }
}

/// Where the solver finds releases: the index and their metadata.
pub trait Packages {
    /// Why a release's metadata couldn't be read.
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

/// The order the search tries a package's versions in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Upstream's.
    NewestFirst,
    /// elm-test's `--dependencies oldest`.
    OldestFirst,
}

/// `verify`: the releases that satisfy `constraints` and everything they
/// need, `None` if there are none.
pub fn verify<P: Packages>(
    packages: &mut P,
    compat: Compat,
    constraints: &BTreeMap<Name, Constraint>,
) -> Result<Option<BTreeMap<Name, Details>>, P::Error> {
    verify_in(packages, compat, constraints, Order::NewestFirst)
}

/// [`verify`], trying each package's versions in `order`.
pub fn verify_in<P: Packages>(
    packages: &mut P,
    compat: Compat,
    constraints: &BTreeMap<Name, Constraint>,
    order: Order,
) -> Result<Option<BTreeMap<Name, Details>>, P::Error> {
    let mut search = Search::new(packages, compat, order);
    let Some(solution) = search.attempt(constraints.clone())? else {
        return Ok(None);
    };
    let details = solution
        .into_iter()
        .map(|(name, version)| {
            let deps = search.releases.take(&name, version).deps;
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
    let (all_directs, all_deps) = directs_and_all(app);
    let relaxations = [
        Relaxation {
            held: &all_deps,
            constraint: &Constraint::exactly,
        },
        Relaxation {
            held: &all_directs,
            constraint: &Constraint::exactly,
        },
        Relaxation {
            held: &all_directs,
            constraint: &|v| Constraint::until_next_minor(v, compat),
        },
        Relaxation {
            held: &all_directs,
            constraint: &|v| Constraint::until_next_major(v, compat),
        },
        Relaxation {
            held: &all_directs,
            constraint: &|_| Constraint::anything(compat),
        },
    ];
    let mut search = Search::new(packages, compat, Order::NewestFirst);
    for Relaxation { held, constraint } in relaxations {
        let mut constraints: BTreeMap<Name, Constraint> = held
            .iter()
            .map(|(name, &version)| (name.clone(), constraint(version)))
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
    let (all_directs, all_deps) = directs_and_all(app);
    let mut search = Search::new(packages, compat, Order::NewestFirst);
    for held in [&all_deps, &all_directs] {
        let mut constraints = BTreeMap::new();
        for (name, &version) in &all_deps {
            let constraint = if free.contains(name) {
                all_directs
                    .contains_key(name)
                    .then(|| Constraint::until_next_major(version, compat))
            } else {
                held.contains_key(name)
                    .then(|| Constraint::exactly(version))
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

/// Dependencies to hold, and how freely to hold each.
struct Relaxation<'a> {
    held: &'a BTreeMap<Name, Version>,
    constraint: &'a dyn Fn(Version) -> Constraint,
}

/// An application's direct dependencies, test ones included, and all its
/// dependencies, as `addToApp` merges them.
fn directs_and_all(app: &AppOutline) -> (BTreeMap<Name, Version>, BTreeMap<Name, Version>) {
    let all_directs = union(&app.direct, &app.test_direct);
    let all_deps = union(&all_directs, &union(&app.indirect, &app.test_indirect));
    (all_directs, all_deps)
}

/// `Map.union`: `a`'s entries, then `b`'s that `a` lacks.
fn union<V: Clone>(a: &BTreeMap<Name, V>, b: &BTreeMap<Name, V>) -> BTreeMap<Name, V> {
    let mut union = b.clone();
    union.extend(a.iter().map(|(k, v)| (k.clone(), v.clone())));
    union
}

/// Why a release's dependencies can't be had.
pub(crate) enum ReleaseError<E> {
    /// Its `elm.json` can't be had, as when offline and not downloaded.
    Unavailable(Name, Version),
    /// Reading it failed.
    Solver(E),
}

/// What a release needs, which must be had.
pub(crate) fn release_deps<P: Packages>(
    packages: &mut P,
    name: &Name,
    version: Version,
) -> Result<BTreeMap<Name, Constraint>, ReleaseError<P::Error>> {
    match packages.release(name, version) {
        Ok(Some(release)) => Ok(release.deps),
        Ok(None) => Err(ReleaseError::Unavailable(name.clone(), version)),
        Err(e) => Err(ReleaseError::Solver(e)),
    }
}

/// `getTransitive`: the packages of `solution` that `roots` need, directly
/// or not, as `deps` says what each needs.
pub(crate) fn reachable<D: Borrow<BTreeMap<Name, Constraint>>, E>(
    solution: &BTreeMap<Name, Version>,
    roots: &BTreeMap<Name, Version>,
    mut deps: impl FnMut(&Name, Version) -> Result<D, E>,
) -> Result<BTreeSet<Name>, E> {
    let mut visited = BTreeSet::new();
    let mut unvisited: Vec<Name> = roots.keys().cloned().collect();
    while let Some(name) = unvisited.pop() {
        if visited.contains(&name) {
            continue;
        }
        let needs = deps(&name, solution[&name])?;
        let needs = needs.borrow().keys();
        unvisited.extend(needs.filter(|dep| solution.contains_key(*dep)).cloned());
        visited.insert(name);
    }
    Ok(visited)
}

/// The metadata of the releases a search read, each asked of `packages`
/// once, which outlives backtracking as upstream's `State` does.
struct Releases<'p, P> {
    packages: &'p mut P,
    read: BTreeMap<(Name, Version), Option<Release>>,
}

const FOUND: &str = "a release in a solution was found";

impl<'p, P: Packages> Releases<'p, P> {
    fn new(packages: &'p mut P) -> Releases<'p, P> {
        Releases {
            packages,
            read: BTreeMap::new(),
        }
    }

    /// `getConstraints`, remembering what it read.
    fn get(&mut self, name: &Name, version: Version) -> Result<Option<&Release>, P::Error> {
        let release = match self.read.entry((name.clone(), version)) {
            Entry::Occupied(read) => read.into_mut(),
            Entry::Vacant(unread) => unread.insert(self.packages.release(name, version)?),
        };
        Ok(release.as_ref())
    }

    /// A release read and found, as each in a solution is.
    fn found(&self, name: &Name, version: Version) -> &Release {
        self.read[&(name.clone(), version)].as_ref().expect(FOUND)
    }

    /// [`Releases::found`], taken out once the search is over.
    fn take(&mut self, name: &Name, version: Version) -> Release {
        self.read
            .remove(&(name.clone(), version))
            .flatten()
            .expect(FOUND)
    }
}

/// The search and the metadata it has read.
struct Search<'p, P> {
    releases: Releases<'p, P>,
    compat: Compat,
    order: Order,
}

/// `Goals`: what is still to pick, and what has been.
#[derive(Clone)]
struct Goals {
    pending: BTreeMap<Name, Constraint>,
    solved: BTreeMap<Name, Version>,
}

impl<'p, P: Packages> Search<'p, P> {
    fn new(packages: &'p mut P, compat: Compat, order: Order) -> Search<'p, P> {
        Search {
            releases: Releases::new(packages),
            compat,
            order,
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
    /// first in name order and its versions in the search's order.
    fn explore(&mut self, mut goals: Goals) -> Result<Option<BTreeMap<Name, Version>>, P::Error> {
        let Some((name, constraint)) = goals.pending.pop_first() else {
            return Ok(Some(goals.solved));
        };
        // `getRelevantVersions`
        let mut versions: Vec<Version> = match self.releases.packages.versions(&name) {
            Some(known) => known.all().filter(|&v| constraint.satisfies(v)).collect(),
            None => Vec::new(),
        };
        match self.order {
            Order::NewestFirst => {}
            Order::OldestFirst => versions.reverse(),
        }
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
        let Some(release) = self.releases.get(name, version)? else {
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
        let direct = pick(new, |name| {
            Some(name) == pkg || app.direct.contains_key(name)
        });
        let Ok(needed) = reachable(new, &direct, |name, version| {
            Ok::<_, Infallible>(&self.releases.found(name, version).deps)
        });
        let indirect = pick(new, |name| {
            needed.contains(name) && !direct.contains_key(name)
        });
        let test_direct = pick(new, |name| {
            Some(name) != pkg && app.test_direct.contains_key(name)
        });
        let test_indirect = pick(new, |name| {
            !direct.contains_key(name)
                && !indirect.contains_key(name)
                && !test_direct.contains_key(name)
        });
        AppOutline {
            elm: app.elm,
            source_dirs: app.source_dirs.clone(),
            direct,
            indirect,
            test_direct,
            test_indirect,
        }
    }
}

/// The releases of `new` whose names `keep` keeps.
fn pick(new: &BTreeMap<Name, Version>, keep: impl Fn(&Name) -> bool) -> BTreeMap<Name, Version> {
    new.iter()
        .filter(|(name, _)| keep(name))
        .map(|(name, &version)| (name.clone(), version))
        .collect()
}
