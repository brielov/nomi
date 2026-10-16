//! The dependencies of elm-test's generated project, as it solves them
//! (`Solve.getDependencies`): the project's direct and test-direct
//! dependencies, held exactly in an application, and what the runner
//! needs, newest first; first from the packages already downloaded, then
//! from the registry.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use nomi_base::{Compat, Report};
use nomi_project::workspace::Workspace;
use nomi_project::{Constraint, Name, Outline, Version};
pub use nomi_registry::solver::Order;
use nomi_registry::solver::{self, Details};
use nomi_registry::workspace::Members;
use nomi_registry::{Http, Known, Packages, Release, Store};

use crate::artifacts::Hash;
use crate::env;
use crate::packages::{self, Resolve};
use crate::paths::{Cache, ELM_JSON};
use crate::report::one_line;

/// The generated project is elm-test's, which says 0.19.3.
const COMPAT: Compat = Compat::V0_19_3;

/// A solution: the dependencies the constraints name, and the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependencies {
    pub direct: BTreeMap<Name, Version>,
    pub indirect: BTreeMap<Name, Version>,
}

/// How elm-test solves: `--dependencies`, whose `oldest` only a
/// package's ranges leave room for, and `--offline`; and whether the
/// packages bundled with nomi are among those it solves from, as they are
/// for `nomi test`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Solve {
    pub order: Order,
    pub offline: bool,
    pub resolve: Resolve,
}

/// The project whose tests run.
#[derive(Clone, Copy, Debug)]
pub struct Tested<'a> {
    pub elm_json: &'a [u8],
    /// `elm_json`, decoded.
    pub outline: &'a Outline,
    /// The workspace it is a member of, whose member packages are each
    /// the one release of its name.
    pub workspace: Option<&'a Workspace>,
}

/// The generated dependencies of `tested`, kept in `dir` by a hash of
/// its `elm.json` as elm-test keeps them by its dependencies; `solving`
/// is called before a solve, which a kept solution saves.
pub fn dependencies(
    tested: Tested<'_>,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
    how: Solve,
    solving: &mut dyn FnMut(),
) -> Result<Dependencies, Report> {
    let Tested {
        elm_json,
        outline,
        workspace,
    } = tested;
    let order = match outline {
        Outline::App(_) => Order::NewestFirst,
        Outline::Pkg(_) => how.order,
    };
    let mut key = elm_json.to_vec();
    if order == Order::OldestFirst {
        key.extend_from_slice(b"\0oldest");
    }
    if how.resolve == Resolve::Nomi {
        key.extend_from_slice(b"\0bundled");
    }
    for member in workspace.iter().flat_map(|w| w.members()) {
        key.push(0);
        key.extend_from_slice(member.dir.as_os_str().as_encoded_bytes());
        key.push(0);
        key.extend_from_slice(&member.outline.write());
    }
    let kept = dir.join(format!("dependencies.{}", Hash::of(&key)));
    if let Some(dependencies) =
        (fs::read_to_string(&kept).ok()).and_then(|t| Dependencies::decode(&t))
    {
        return Ok(dependencies);
    }
    solving();
    let dependencies = solve(outline, workspace, cache, http, Solve { order, ..how })?;
    // A solution that can't be kept is solved again next time.
    let _ = fs::write(&kept, dependencies.encode());
    Ok(dependencies)
}

/// What the generated project needs: the project's dependencies, its
/// tests', and the runner's.
fn constraints(outline: &Outline) -> Result<BTreeMap<Name, Constraint>, Report> {
    let (mut constraints, tests): (BTreeMap<Name, Constraint>, _) = match outline {
        Outline::App(app) => {
            let exactly = (app.direct.iter().chain(&app.test_direct))
                .map(|(name, &version)| (name.clone(), Constraint::exactly(version)));
            (exactly.collect(), BTreeMap::new())
        }
        Outline::Pkg(pkg) => (pkg.deps.clone(), pkg.test_deps.clone()),
    };
    let runner = [Name::core(), Name::json(), Name::time(), Name::random()]
        .map(|name| (name, Constraint::until_next_major(Version::ONE, COMPAT)));
    for (name, constraint) in tests.into_iter().chain(runner) {
        let both = match constraints.get(&name) {
            Some(held) => held.intersect(&constraint),
            None => Some(constraint),
        };
        let Some(both) = both else {
            return Err(no_solution(&format!(
                "The tests need {name} {constraint}, which the project's elm.json rules out."
            )));
        };
        constraints.insert(name, both);
    }
    Ok(constraints)
}

fn solve(
    outline: &Outline,
    workspace: Option<&Workspace>,
    cache: &Cache,
    http: &impl Http,
    how: Solve,
) -> Result<Dependencies, Report> {
    let constraints = constraints(outline)?;
    let store = packages::store_for(cache, how.resolve)?;
    let split = |solution: BTreeMap<Name, Details>| {
        let (direct, indirect) = (solution.into_iter())
            .map(|(name, details)| (name, details.version))
            .partition(|(name, _)| constraints.contains_key(name));
        Dependencies { direct, indirect }
    };
    let mut local = Local::new(&store).map_err(|e| io_problem(&e))?;
    let found = match workspace {
        None => solver::verify_in(&mut local, COMPAT, &constraints, how.order),
        Some(workspace) => {
            let mut local = Members::new(workspace, &mut local);
            solver::verify_in(&mut local, COMPAT, &constraints, how.order)
        }
    };
    if let Ok(Some(solution)) = found {
        return Ok(split(solution));
    }
    if how.offline {
        return Err(no_solution(
            "I could not find versions of the project's dependencies and the test runner's that work together among the packages already downloaded, and --offline keeps me from looking further.",
        ));
    }
    let mut registry = env::registry(
        cache,
        &store,
        http,
        COMPAT,
        "I need the list of published packages to find the tests' dependencies",
    )?;
    let found = match workspace {
        None => solver::verify_in(&mut registry, COMPAT, &constraints, how.order),
        Some(workspace) => {
            let mut registry = Members::new(workspace, &mut registry);
            solver::verify_in(&mut registry, COMPAT, &constraints, how.order)
        }
    };
    match found {
        Ok(Some(solution)) => Ok(split(solution)),
        Ok(None) => Err(no_solution(
            "I could not find versions of the project's dependencies and the test runner's that work together.",
        )),
        Err(problem) => Err(nomi_registry::report::solver_problem(&problem)),
    }
}

fn no_solution(message: &str) -> Report {
    one_line("NO SOLUTION FOR THE TESTS", message)
}

fn io_problem(e: &io::Error) -> Report {
    one_line(
        "PROBLEM READING PACKAGES",
        &format!("I could not read the packages nomi has downloaded: {e}"),
    )
}

/// The packages already downloaded, which elm-test solves from first.
struct Local<'a> {
    store: &'a Store,
    known: BTreeMap<Name, Known>,
}

impl<'a> Local<'a> {
    fn new(store: &'a Store) -> io::Result<Local<'a>> {
        let known = store.downloaded()?;
        Ok(Local { store, known })
    }
}

impl Packages for Local<'_> {
    type Error = io::Error;

    fn versions(&self, name: &Name) -> Option<&Known> {
        self.known.get(name)
    }

    fn release(&mut self, name: &Name, version: Version) -> io::Result<Option<Release>> {
        let Some(dir) = self.store.get(name, version)? else {
            return Ok(None);
        };
        Ok(Release::decode(&fs::read(dir.join(ELM_JSON))?, COMPAT))
    }

    fn offline(&self) -> bool {
        true
    }
}

impl Dependencies {
    /// The solution as it is kept: a line for each dependency, `direct`
    /// or `indirect`.
    fn encode(&self) -> String {
        let lines = |kind: &str, deps: &BTreeMap<Name, Version>| -> String {
            (deps.iter())
                .map(|(name, version)| format!("{kind} {name} {version}\n"))
                .collect()
        };
        lines("direct", &self.direct) + &lines("indirect", &self.indirect)
    }

    fn decode(text: &str) -> Option<Dependencies> {
        let mut dependencies = Dependencies::default();
        for line in text.lines() {
            let mut parts = line.split(' ');
            let deps = match parts.next()? {
                "direct" => &mut dependencies.direct,
                "indirect" => &mut dependencies.indirect,
                _ => return None,
            };
            deps.insert(parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
        }
        Some(dependencies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_solutions_read_back() {
        let name = |s: &str| s.parse::<Name>().unwrap();
        let dependencies = Dependencies {
            direct: BTreeMap::from([(name("elm/core"), Version::new(1, 0, 5))]),
            indirect: BTreeMap::from([(name("elm/html"), Version::new(1, 0, 1))]),
        };
        assert_eq!(
            Dependencies::decode(&dependencies.encode()),
            Some(dependencies)
        );
        assert_eq!(Dependencies::decode("both elm/core 1.0.5\n"), None);
    }
}
