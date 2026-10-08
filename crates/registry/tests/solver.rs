//! The solver ratchet: each scenario under `solver/scenarios/` and in
//! `solver/generated.json`, for each Compat, is a case `<scenario>@<compat>`.
//! It passes when nomi answers as upstream's solver did, which
//! `solver/oracle.txt` records (`cargo xtask oracle solver`).
//!
//! A scenario is a registry, the metadata of its releases and a project,
//! with perhaps a package to add; `solver/README.md` describes the format
//! and the answers.

use std::collections::BTreeMap;
use std::path::Path;

use nomi_base::Compat;
use nomi_project::json::Cursor;
use nomi_project::{Name, Outline, Version};
use nomi_registry::install::{self, InstallProblem, Plan};
use nomi_registry::{DetailsProblem, Known, Packages, Release, details, solver};
use nomi_testkit::ratchet::Suite;
use serde::Deserialize;
use serde_json::json;

const RERUN: &str = "rerun `cargo xtask oracle solver`";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    universe: Option<String>,
    #[serde(default)]
    registry: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    releases: BTreeMap<String, ScenarioRelease>,
    project: Project,
    add: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Universe {
    registry: BTreeMap<String, Vec<String>>,
    releases: BTreeMap<String, ScenarioRelease>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioRelease {
    elm: String,
    #[serde(default)]
    deps: BTreeMap<String, String>,
    #[serde(default = "yes")]
    src: bool,
    #[serde(default)]
    broken: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum Project {
    #[serde(rename_all = "kebab-case")]
    Application {
        elm: Option<String>,
        #[serde(default)]
        direct: BTreeMap<String, String>,
        #[serde(default)]
        indirect: BTreeMap<String, String>,
        #[serde(default)]
        test_direct: BTreeMap<String, String>,
        #[serde(default)]
        test_indirect: BTreeMap<String, String>,
    },
    #[serde(rename_all = "kebab-case")]
    Package {
        elm: String,
        #[serde(default)]
        deps: BTreeMap<String, String>,
        #[serde(default)]
        test_deps: BTreeMap<String, String>,
    },
}

fn yes() -> bool {
    true
}

#[test]
fn solver() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/solver");
    let scenarios = scenarios(&dir);
    let text = std::fs::read_to_string(dir.join("oracle.txt"))
        .unwrap_or_else(|e| panic!("reading oracle.txt: {e}; {RERUN}"));
    let mut answers = answers(&text);
    let mut universes = BTreeMap::new();
    let suite = Suite::open(&dir);
    let mut outcomes = Vec::new();
    for (name, scenario) in &scenarios {
        let universe = universe(&dir, &mut universes, scenario);
        for compat in Compat::ALL {
            let case = format!("{name}@{compat}");
            let expected = answers
                .remove(&case)
                .unwrap_or_else(|| panic!("{case}: no answer in oracle.txt; {RERUN}"));
            let actual = answer(&universe, scenario, compat);
            let outcome = if actual == expected {
                Ok(())
            } else {
                Err(format!("upstream:\n{expected}nomi:\n{actual}"))
            };
            outcomes.push((case, outcome));
        }
    }
    assert!(
        answers.is_empty(),
        "oracle.txt answers cases no scenario has: {:?}; {RERUN}",
        answers.keys().collect::<Vec<_>>()
    );
    suite.finish(outcomes);
}

/// Whenever a package's dependencies have no solution, PubGrub says why,
/// and it finds none exactly when the search finds none.
#[test]
fn explanations() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/solver");
    let compat = Compat::LATEST;
    let mut universes = BTreeMap::new();
    let mut explanations = String::new();
    for (name, scenario) in &scenarios(&dir) {
        let universe = universe(&dir, &mut universes, scenario);
        let src = outline(&scenario.project, compat).to_string();
        let (None, Ok(Outline::Pkg(pkg))) = (&scenario.add, Outline::read(src.as_bytes(), compat))
        else {
            continue;
        };
        let mut constraints = pkg.test_deps.clone();
        constraints.extend(pkg.deps.clone());
        let Ok(solution) =
            solver::verify(&mut Fixture::new(&universe, compat), compat, &constraints)
        else {
            continue;
        };
        let explanation =
            solver::explain(&mut Fixture::new(&universe, compat), compat, &constraints);
        assert_eq!(solution.is_none(), explanation.is_some(), "{name}");
        if let Some(explanation) = explanation {
            explanations.push_str(&format!("== {name}\n{explanation}\n\n"));
        }
    }
    insta::assert_snapshot!(explanations);
}

/// What nomi answers, in the oracle's words.
fn answer(universe: &Universe, scenario: &Scenario, compat: Compat) -> String {
    let mut packages = Fixture::new(universe, compat);
    let src = outline(&scenario.project, compat).to_string();
    let Ok(outline) = Outline::read(src.as_bytes(), compat) else {
        return "bad-outline\n".to_owned();
    };
    let lines = match &scenario.add {
        None => match details::verify(&mut packages, compat, &outline) {
            Ok(solution) => ok(solution.iter().map(|(n, d)| release("", n, d.version))),
            Err(DetailsProblem::AppElm(_) | DetailsProblem::PkgElm(_)) => vec!["bad-elm".into()],
            Err(DetailsProblem::HandEdited) => vec!["hand-edited".into()],
            Err(DetailsProblem::NoSolution) => vec!["no-offline-solution".into()],
            Err(DetailsProblem::Solver(Unreadable)) => vec!["error".into()],
        },
        Some(pkg) => match install::plan(&mut packages, compat, &name(pkg, compat), &outline) {
            Ok(Plan::AlreadyInstalled) => vec!["already-installed".into()],
            Ok(Plan::PromoteIndirect(_)) => vec!["promote-indirect".into()],
            Ok(Plan::PromoteTest(_)) => vec!["promote-test".into()],
            Ok(Plan::App { outline, .. }) => ok([
                ("direct ", &outline.direct),
                ("indirect ", &outline.indirect),
                ("test-direct ", &outline.test_direct),
                ("test-indirect ", &outline.test_indirect),
            ]
            .into_iter()
            .flat_map(|(label, deps)| deps.iter().map(move |(n, &v)| release(label, n, v)))),
            Ok(Plan::Pkg { solution, .. }) => ok(solution.iter().map(|(n, &v)| release("", n, v))),
            Err(InstallProblem::UnknownPackage) => vec!["unknown-package".into()],
            Err(InstallProblem::NoSolution) => vec!["no-offline-solution".into()],
            Err(InstallProblem::Solver(Unreadable)) => vec!["error".into()],
        },
    };
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn ok(lines: impl Iterator<Item = String>) -> Vec<String> {
    std::iter::once("ok".to_owned()).chain(lines).collect()
}

fn release(label: &str, name: &Name, version: Version) -> String {
    format!("{label}{name} {version}")
}

fn name(s: &str, compat: Compat) -> Name {
    Name::parse(s.as_bytes(), Cursor::start(compat), compat).unwrap()
}

fn version(s: &str, compat: Compat) -> Version {
    Version::parse(s.as_bytes(), Cursor::start(compat), compat).unwrap()
}

/// The registry and package cache, offline as the oracle runs.
struct Fixture {
    compat: Compat,
    registry: BTreeMap<Name, Known>,
    /// Each cached release's `elm.json`, and whether its sources are.
    cache: BTreeMap<(Name, Version), (String, bool)>,
}

/// An `elm.json` upstream can't read.
#[derive(Debug)]
struct Unreadable;

impl Fixture {
    fn new(universe: &Universe, compat: Compat) -> Fixture {
        let registry = universe
            .registry
            .iter()
            .map(|(n, versions)| {
                let mut versions = versions.iter().map(|v| version(v, compat));
                let newest = versions.next().unwrap();
                let known = Known {
                    newest,
                    previous: versions.collect(),
                };
                (name(n, compat), known)
            })
            .collect();
        let cache = universe
            .releases
            .iter()
            .map(|(key, release)| {
                let (n, v) = key.split_once('@').unwrap();
                let text = if release.broken {
                    "{}".to_owned()
                } else {
                    json!({
                        "type": "package",
                        "name": n,
                        "summary": "fixture",
                        "license": "BSD-3-Clause",
                        "version": v,
                        "exposed-modules": ["Fixture"],
                        "elm-version": release.elm,
                        "dependencies": release.deps,
                        "test-dependencies": {},
                    })
                    .to_string()
                };
                ((name(n, compat), version(v, compat)), (text, release.src))
            })
            .collect();
        Fixture {
            compat,
            registry,
            cache,
        }
    }
}

impl Packages for Fixture {
    type Error = Unreadable;

    fn versions(&self, name: &Name) -> Option<&Known> {
        self.registry.get(name)
    }

    /// `getConstraints`, offline.
    fn release(&mut self, name: &Name, version: Version) -> Result<Option<Release>, Unreadable> {
        let Some((text, src)) = self.cache.get(&(name.clone(), version)) else {
            return Ok(None);
        };
        let release = Release::decode(text.as_bytes(), self.compat).ok_or(Unreadable)?;
        Ok(src.then_some(release))
    }
}

/// The project's `elm.json`, as the oracle writes it.
fn outline(project: &Project, compat: Compat) -> serde_json::Value {
    match project {
        Project::Application {
            elm,
            direct,
            indirect,
            test_direct,
            test_indirect,
        } => json!({
            "type": "application",
            "source-directories": ["src"],
            "elm-version": elm.clone().unwrap_or_else(|| compat.to_string()),
            "dependencies": { "direct": direct, "indirect": indirect },
            "test-dependencies": { "direct": test_direct, "indirect": test_indirect },
        }),
        Project::Package {
            elm,
            deps,
            test_deps,
        } => json!({
            "type": "package",
            "name": "nomi/root",
            "summary": "fixture",
            "license": "BSD-3-Clause",
            "version": "1.0.0",
            "exposed-modules": ["Main"],
            "elm-version": elm,
            "dependencies": deps,
            "test-dependencies": test_deps,
        }),
    }
}

/// The scenario's registry and releases, over its universe's.
fn universe(
    dir: &Path,
    universes: &mut BTreeMap<String, Universe>,
    scenario: &Scenario,
) -> Universe {
    let mut universe = match &scenario.universe {
        Some(name) => universes
            .entry(name.clone())
            .or_insert_with(|| {
                let path = dir.join("universes").join(format!("{name}.json"));
                let text = std::fs::read_to_string(&path).unwrap();
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            })
            .clone(),
        None => Universe::default(),
    };
    universe.registry.extend(scenario.registry.clone());
    universe.releases.extend(scenario.releases.clone());
    universe
}

/// Every scenario, by name.
fn scenarios(dir: &Path) -> BTreeMap<String, Scenario> {
    let text = std::fs::read_to_string(dir.join("generated.json"))
        .unwrap_or_else(|e| panic!("reading generated.json: {e}; {RERUN}"));
    let mut scenarios: BTreeMap<String, Scenario> =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("generated.json: {e}"));
    for entry in std::fs::read_dir(dir.join("scenarios")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let scenario =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            scenarios.insert(name.clone(), scenario).is_none(),
            "{name}: both generated and written"
        );
    }
    scenarios
}

/// `oracle.txt`'s answers by case: after a header of comments, each case
/// is `== <case>` and the lines upstream printed.
fn answers(text: &str) -> BTreeMap<String, String> {
    let mut answers = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some(case) = line.strip_prefix("== ") {
            answers.extend(current.take());
            current = Some((case.to_owned(), String::new()));
        } else if let Some((_, answer)) = &mut current
            && !line.is_empty()
        {
            answer.push_str(line);
            answer.push('\n');
        }
    }
    answers.extend(current);
    answers
}
