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
use nomi_project::outline::AppOutline;
use nomi_project::{Name, Outline, OutlineError, Version};
use nomi_registry::install::{self, Change, InstallProblem, Plan};
use nomi_registry::remove::{self, RemoveProblem};
use nomi_registry::update::{self, UpdateProblem};
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

/// Whenever a package's dependencies have no solution, `PubGrub` says why,
/// and it finds none exactly when the search finds none.
#[test]
fn explanations() {
    let compat = Compat::LATEST;
    let mut explanations = String::new();
    for Case {
        name,
        universe,
        scenario,
        outline,
    } in &cases()
    {
        let (None, Ok(Outline::Pkg(pkg))) = (&scenario.add, outline) else {
            continue;
        };
        let mut constraints = pkg.test_deps.clone();
        constraints.extend(pkg.deps.clone());
        let Ok(solution) =
            solver::verify(&mut Fixture::new(universe, compat), compat, &constraints)
        else {
            continue;
        };
        let explanation =
            solver::explain(&mut Fixture::new(universe, compat), compat, &constraints);
        assert_eq!(solution.is_none(), explanation.is_some(), "{name}");
        if let Some(explanation) = explanation {
            explanations.push_str(&format!("== {name}\n{explanation}\n\n"));
        }
    }
    insta::assert_snapshot!(explanations);
}

/// Removing the package each scenario adds to an application leaves its
/// other dependencies at their versions, and elm accepts the result. When
/// adding only inserted packages, removing gives back the application
/// the scenario started from, unless that kept packages nothing needed.
#[test]
fn removing_undoes_adding() {
    let compat = Compat::LATEST;
    let (mut checked, mut restored) = (0, 0);
    for Case {
        name,
        universe,
        scenario,
        outline,
    } in &cases()
    {
        let (Some(pkg), Ok(Outline::App(before))) = (&scenario.add, outline) else {
            continue;
        };
        let pkg = self::name(pkg);
        let mut packages = Fixture::new(universe, compat);
        let Ok(Plan::App { changes, outline }) =
            install::plan(&mut packages, compat, &pkg, &Outline::App(before.clone()))
        else {
            continue;
        };
        let after = match remove::plan(&mut packages, &pkg, &Outline::App(outline.clone())) {
            Ok(remove::Plan::App { outline, .. }) => outline,
            Err(RemoveProblem::Unavailable(..)) => continue,
            other => panic!("{name}: {other:?}"),
        };
        checked += 1;
        let mut expected_direct = outline.direct.clone();
        expected_direct.remove(&pkg);
        assert_eq!(after.direct, expected_direct, "{name}");
        for deps in [&after.indirect, &after.test_direct, &after.test_indirect] {
            for (dep, version) in deps {
                let kept = [
                    &outline.direct,
                    &outline.indirect,
                    &outline.test_direct,
                    &outline.test_indirect,
                ]
                .iter()
                .find_map(|deps| deps.get(dep));
                assert_eq!(kept, Some(version), "{name}: {dep}");
            }
        }
        let after = Outline::App(after);
        assert!(
            details::verify(&mut packages, compat, &after).is_ok(),
            "{name}"
        );
        let Outline::App(after) = after else {
            unreachable!()
        };
        if changes.values().all(|c| matches!(c, Change::Insert(_))) && tidy(&mut packages, before) {
            assert_eq!(&after, before, "{name}");
            restored += 1;
        }
    }
    assert!(checked > 50, "checked only {checked} scenarios");
    assert!(restored > 10, "restored only {restored} scenarios");
}

/// Updating an application elm accepts keeps its direct and test
/// dependencies, moves none of them back or past its major version, and
/// leaves one elm accepts, which a second update doesn't change. Updating
/// one dependency moves no direct one that doesn't need it.
#[test]
fn updating_stays_within_majors() {
    let compat = Compat::LATEST;
    let (mut checked, mut moved) = (0, 0);
    for Case {
        name,
        universe,
        outline,
        ..
    } in &cases()
    {
        let Ok(Outline::App(before)) = outline else {
            continue;
        };
        let mut packages = Fixture::new(universe, compat);
        if details::verify(&mut packages, compat, &Outline::App(before.clone())).is_err() {
            continue;
        }
        let update = |packages: &mut Fixture, pkg: Option<&Name>, app| match update::plan(
            packages,
            compat,
            pkg,
            &Outline::App(app),
        ) {
            Ok(update::Plan::App { changes, outline }) => Some((changes, outline)),
            Err(UpdateProblem::Unavailable(..) | UpdateProblem::NoSolution) => None,
            other => panic!("{name}: {other:?}"),
        };
        let Some((changes, after)) = update(&mut packages, None, before.clone()) else {
            continue;
        };
        checked += 1;
        for (old, new) in [
            (&before.direct, &after.direct),
            (&before.test_direct, &after.test_direct),
        ] {
            assert!(old.keys().eq(new.keys()), "{name}");
            for (dep, version) in old {
                assert!(new[dep] >= *version, "{name}: {dep}");
                assert_eq!(new[dep].major, version.major, "{name}: {dep}");
            }
        }
        let after_outline = Outline::App(after.clone());
        assert!(
            details::verify(&mut packages, compat, &after_outline).is_ok(),
            "{name}"
        );
        let again = update(&mut packages, None, after.clone());
        assert!(
            again.is_some_and(|(changes, _)| changes.is_empty()),
            "{name}"
        );
        if !changes.is_empty() {
            moved += 1;
        }
        for pkg in before.direct.keys() {
            let Some((_, one)) = update(&mut packages, Some(pkg), before.clone()) else {
                continue;
            };
            assert!(
                details::verify(&mut packages, compat, &Outline::App(one.clone())).is_ok(),
                "{name}: {pkg}"
            );
            let all = [
                &before.direct,
                &before.indirect,
                &before.test_direct,
                &before.test_indirect,
            ];
            for (dep, &version) in before.direct.iter().chain(&before.test_direct) {
                let moved = [&one.direct, &one.test_direct]
                    .iter()
                    .find_map(|deps| deps.get(dep))
                    != Some(&version);
                assert!(
                    !moved || reaches(&mut packages, &all, dep, pkg),
                    "{name}: updating {pkg} moved {dep}"
                );
            }
        }
    }
    assert!(checked >= 50, "checked only {checked} scenarios");
    assert!(moved > 10, "moved only {moved} scenarios");
}

/// Whether `from`, at its version among `deps`, needs `to`, directly or
/// not, or is it.
fn reaches(
    packages: &mut Fixture,
    deps: &[&BTreeMap<Name, Version>],
    from: &Name,
    to: &Name,
) -> bool {
    let version = |name: &Name| deps.iter().find_map(|deps| deps.get(name)).copied();
    let mut visited = std::collections::BTreeSet::new();
    let mut unvisited = vec![from.clone()];
    while let Some(name) = unvisited.pop() {
        if &name == to {
            return true;
        }
        if let Some(v) = version(&name)
            && visited.insert(name.clone())
            && let Ok(Some(release)) = packages.release(&name, v)
        {
            unvisited.extend(release.deps.into_keys());
        }
    }
    false
}

/// Whether elm accepts `app` and removing a package nothing depends on
/// would keep all the rest: every indirect dependency is needed.
fn tidy(packages: &mut Fixture, app: &AppOutline) -> bool {
    let compat = Compat::LATEST;
    if details::verify(packages, compat, &Outline::App(app.clone())).is_err() {
        return false;
    }
    let unused = name("nomi/unused");
    let mut with = app.clone();
    with.direct.insert(unused.clone(), version("1.0.0", compat));
    packages.cache.insert(
        (unused.clone(), version("1.0.0", compat)),
        Cached {
            elm_json: json!({
                "type": "package", "name": "nomi/unused", "summary": "fixture",
                "license": "BSD-3-Clause", "version": "1.0.0", "exposed-modules": ["A"],
                "elm-version": "0.19.0 <= v < 0.20.0", "dependencies": {},
                "test-dependencies": {},
            })
            .to_string(),
            sources: true,
        },
    );
    matches!(
        remove::plan(packages, &unused, &Outline::App(with)),
        Ok(remove::Plan::App { outline, .. }) if &outline == app
    )
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
            Err(DetailsProblem::NoOfflineSolution) => vec!["no-offline-solution".into()],
            Err(DetailsProblem::NoSolution) => vec!["no-solution".into()],
            Err(DetailsProblem::Solver(Unreadable)) => vec!["error".into()],
        },
        Some(pkg) => match install::plan(&mut packages, compat, &name(pkg), &outline) {
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

fn name(s: &str) -> Name {
    s.parse().unwrap()
}

fn version(s: &str, compat: Compat) -> Version {
    Version::parse(s.as_bytes(), Cursor::start(compat), compat).unwrap()
}

/// The registry and package cache, offline as the oracle runs.
struct Fixture {
    compat: Compat,
    registry: BTreeMap<Name, Known>,
    cache: BTreeMap<(Name, Version), Cached>,
}

/// A release in the package cache.
struct Cached {
    elm_json: String,
    /// Whether its sources are cached too.
    sources: bool,
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
                (name(n), known)
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
                let cached = Cached {
                    elm_json: text,
                    sources: release.src,
                };
                ((name(n), version(v, compat)), cached)
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
        let Some(cached) = self.cache.get(&(name.clone(), version)) else {
            return Ok(None);
        };
        let release = Release::decode(cached.elm_json.as_bytes(), self.compat).ok_or(Unreadable)?;
        Ok(cached.sources.then_some(release))
    }

    /// The oracle runs elm offline.
    fn offline(&self) -> bool {
        true
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

/// A scenario, with its registry and releases and its project as the
/// newest release reads it.
struct Case {
    name: String,
    universe: Universe,
    scenario: Scenario,
    outline: Result<Outline, OutlineError>,
}

/// Every scenario as a [`Case`].
fn cases() -> Vec<Case> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/solver");
    let compat = Compat::LATEST;
    let mut universes = BTreeMap::new();
    (scenarios(&dir).into_iter())
        .map(|(name, scenario)| {
            let universe = universe(&dir, &mut universes, &scenario);
            let src = outline(&scenario.project, compat).to_string();
            let outline = Outline::read(src.as_bytes(), compat);
            Case {
                name,
                universe,
                scenario,
                outline,
            }
        })
        .collect()
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
