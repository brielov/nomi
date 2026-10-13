//! The dependencies of elm-test's generated project (plan 10.3), as it
//! solves them
//! (`Solve.getDependencies`): the project's direct and test-direct
//! dependencies, held exactly in an application, and what the runner
//! needs, newest first; first from the packages already downloaded, then
//! from the registry.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use crate::artifacts::Hash;
use crate::env;
use crate::paths::Cache;
use nomi_base::{Compat, Report};
use nomi_project::json::Cursor;
use nomi_project::{Constraint, Name, Outline, Version};
use nomi_registry::solver::{self, Details};
use nomi_registry::{Http, Known, Packages, Release, Store};

/// The generated project is elm-test's, which says 0.19.3.
const COMPAT: Compat = Compat::V0_19_3;

/// A solution: the dependencies the constraints name, and the rest.
pub type Dependencies = (BTreeMap<Name, Version>, BTreeMap<Name, Version>);

/// The generated dependencies of the project whose `elm.json` is
/// `elm_json`, decoded as `outline`, kept in `dir` by a hash of it as
/// elm-test keeps them by its dependencies.
pub fn dependencies(
    elm_json: &[u8],
    outline: &Outline,
    dir: &Path,
    cache: &Cache,
    http: &impl Http,
) -> Result<Dependencies, Report> {
    let hex: String = (Hash::of(elm_json).0.iter())
        .map(|b| format!("{b:02x}"))
        .collect();
    let kept = dir.join(format!("dependencies.{hex}"));
    if let Some(dependencies) = fs::read_to_string(&kept).ok().and_then(|t| read(&t)) {
        return Ok(dependencies);
    }
    let dependencies = solve(outline, cache, http)?;
    // A solution that can't be kept is solved again next time.
    let _ = fs::write(&kept, write(&dependencies));
    Ok(dependencies)
}

/// What the generated project needs: the project's dependencies, its
/// tests', and the runner's.
fn constraints(outline: &Outline) -> Result<BTreeMap<Name, Constraint>, Report> {
    let mut constraints: BTreeMap<Name, Constraint> = match outline {
        Outline::App(app) => (app.direct.iter().chain(&app.test_direct))
            .map(|(name, &version)| (name.clone(), Constraint::exactly(version)))
            .collect(),
        Outline::Pkg(pkg) => pkg.deps.clone(),
    };
    let tests = match outline {
        Outline::App(_) => BTreeMap::new(),
        Outline::Pkg(pkg) => pkg.test_deps.clone(),
    };
    let runner = [Name::core(), Name::json(), Name::time(), Name::random()].map(|name| {
        (
            name,
            Constraint::until_next_major(Version::new(1, 0, 0), COMPAT),
        )
    });
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

fn solve(outline: &Outline, cache: &Cache, http: &impl Http) -> Result<Dependencies, Report> {
    let constraints = constraints(outline)?;
    let store = Store::new(cache.packages());
    let split = |solution: BTreeMap<Name, Details>| -> Dependencies {
        (solution.into_iter())
            .map(|(name, details)| (name, details.version))
            .partition(|(name, _)| constraints.contains_key(name))
    };
    let mut local = Local::new(&store).map_err(io_problem)?;
    if let Ok(Some(solution)) = solver::verify(&mut local, COMPAT, &constraints) {
        return Ok(split(solution));
    }
    let mut registry = env::registry(
        cache,
        &store,
        http,
        COMPAT,
        "I need the list of published packages to find the tests' dependencies",
    )?;
    match solver::verify(&mut registry, COMPAT, &constraints) {
        Ok(Some(solution)) => Ok(split(solution)),
        Ok(None) => Err(no_solution(
            "I could not find versions of the project's dependencies and the test runner's that work together.",
        )),
        Err(problem) => Err(nomi_registry::report::solver_problem(&problem)),
    }
}

fn no_solution(message: &str) -> Report {
    Report::new("NO SOLUTION FOR THE TESTS", None, message, [])
}

fn io_problem(e: io::Error) -> Report {
    Report::new(
        "PROBLEM READING PACKAGES",
        None,
        &format!("I could not read the packages nomi has downloaded: {e}"),
        [],
    )
}

/// The packages already downloaded, which elm-test solves from first.
struct Local<'a> {
    store: &'a Store,
    known: BTreeMap<Name, Known>,
}

impl<'a> Local<'a> {
    fn new(store: &'a Store) -> io::Result<Local<'a>> {
        let mut known = BTreeMap::new();
        for (name, mut versions) in store.downloaded()? {
            let Ok(name) = Name::parse(name.as_bytes(), Cursor::start(COMPAT), COMPAT) else {
                continue;
            };
            versions.reverse();
            if let Some((&newest, previous)) = versions.split_first() {
                let previous = previous.to_vec();
                known.insert(name, Known { newest, previous });
            }
        }
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
        Ok(Release::decode(&fs::read(dir.join("elm.json"))?, COMPAT))
    }

    fn offline(&self) -> bool {
        true
    }
}

/// A kept solution: a line for each dependency, `direct` or `indirect`.
fn write((direct, indirect): &Dependencies) -> String {
    let lines = |kind: &str, deps: &BTreeMap<Name, Version>| -> String {
        (deps.iter())
            .map(|(name, version)| format!("{kind} {name} {version}\n"))
            .collect()
    };
    lines("direct", direct) + &lines("indirect", indirect)
}

fn read(text: &str) -> Option<Dependencies> {
    let (mut direct, mut indirect) = (BTreeMap::new(), BTreeMap::new());
    for line in text.lines() {
        let mut parts = line.split(' ');
        let deps = match parts.next()? {
            "direct" => &mut direct,
            "indirect" => &mut indirect,
            _ => return None,
        };
        let name = Name::parse(parts.next()?.as_bytes(), Cursor::start(COMPAT), COMPAT).ok()?;
        let version =
            Version::parse(parts.next()?.as_bytes(), Cursor::start(COMPAT), COMPAT).ok()?;
        deps.insert(name, version);
    }
    Some((direct, indirect))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_solutions_read_back() {
        let name = |s: &str| Name::parse(s.as_bytes(), Cursor::start(COMPAT), COMPAT).unwrap();
        let dependencies = (
            BTreeMap::from([(name("elm/core"), Version::new(1, 0, 5))]),
            BTreeMap::from([(name("elm/html"), Version::new(1, 0, 1))]),
        );
        assert_eq!(read(&write(&dependencies)), Some(dependencies));
        assert_eq!(read("both elm/core 1.0.5\n"), None);
    }
}
