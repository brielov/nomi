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
use nomi_testkit::ratchet::Suite;
use serde::Deserialize;

const RERUN: &str = "rerun `cargo xtask oracle solver`";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Scenario {
    universe: Option<String>,
    #[serde(default)]
    registry: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    releases: BTreeMap<String, Release>,
    project: Project,
    add: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Release {
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
#[allow(dead_code)]
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
    let suite = Suite::open(&dir);
    let mut outcomes = Vec::new();
    for name in scenarios.keys() {
        for compat in Compat::ALL {
            let case = format!("{name}@{compat}");
            let _answer = answers
                .remove(&case)
                .unwrap_or_else(|| panic!("{case}: no answer in oracle.txt; {RERUN}"));
            outcomes.push((case, Err("nomi has no solver yet".to_owned())));
        }
    }
    assert!(
        answers.is_empty(),
        "oracle.txt answers cases no scenario has: {:?}; {RERUN}",
        answers.keys().collect::<Vec<_>>()
    );
    suite.finish(outcomes);
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
