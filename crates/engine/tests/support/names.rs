//! The oracle's projects (`cargo xtask oracle names`, `oracle types` and
//! the like): the curated cases, the seeds, and the seeds' mutations,
//! each run as every release, and the two ratchets each suite of them
//! has, of whole reports and of titles.

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_engine::check::Goal;
use nomi_testkit::edit;
use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::{self, Case};
use serde_json::Value;

use crate::projects::{self, CRASH, compat_name, same_json, shown, table};

/// The ratchet of the suite `suite`'s whole reports: each project as each
/// release, passing when nomi's `--report=json` and terminal report are
/// elm's, and, as `goal` asks, its `docs.json`.
pub fn reports_ratchet(suite: &str, goal: Goal) {
    let dir = suite_dir(suite);
    let outcomes = runs(&dir, goal).map(|run| {
        let outcome = if !same_json(&run.elm_json, &run.json) {
            Err(format!(
                "--report=json:\nnomi: {}\nelm: {}",
                run.json, run.elm_json
            ))
        } else if run.terminal != run.elm_terminal {
            Err(format!(
                "terminal:\nnomi:\n{}\nelm:\n{}",
                run.terminal, run.elm_terminal
            ))
        } else if run.docs != run.elm_docs {
            Err(format!(
                "docs.json:\nnomi: {:?}\nelm: {:?}",
                run.docs, run.elm_docs
            ))
        } else {
            Ok(())
        };
        shown(run.id, outcome)
    });
    Suite::open(dir.with_file_name(format!("{suite}_reports"))).finish(outcomes);
}

/// The ratchet of the suite `suite`'s titles: each project as each
/// release, passing when nomi reports the problems elm does, by title and
/// region, in each module elm names.
pub fn titles_ratchet(suite: &str) {
    let dir = suite_dir(suite);
    let outcomes = runs(&dir, Goal::Check).map(|run| {
        let (elm, nomi) = (titles(&run.elm_json), titles(&run.json));
        let outcome = if elm == nomi {
            Ok(())
        } else {
            Err(format!("nomi: {nomi}\nelm: {elm}"))
        };
        shown(run.id, outcome)
    });
    Suite::open(dir.with_file_name(format!("{suite}_titles"))).finish(outcomes);
}

/// A report reduced to what a titles ratchet holds nomi to: the project's
/// problem's title, or each module's path and its problems' titles and
/// regions.
fn titles(json: &str) -> Value {
    if json.is_empty() || json == "accepted" {
        return Value::Null;
    }
    let report: Value = serde_json::from_str(json).unwrap();
    if report["type"] == "error" {
        return report["title"].clone();
    }
    let modules = report["errors"].as_array().unwrap().iter().map(|module| {
        let problems = (module["problems"].as_array().unwrap().iter())
            .map(|p| serde_json::json!([p["title"], p["region"]]));
        serde_json::json!([module["path"], problems.collect::<Vec<_>>()])
    });
    Value::Array(modules.collect())
}

/// The directory of the suite `name`.
fn suite_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

/// Each project by its id: `case/…`, `seed/…` and `mutation/…`.
fn projects(dir: &Path) -> Vec<(String, Case)> {
    let cases_in = |sub: &str| transcript::cases(&dir.join(sub));
    let mut projects: Vec<(String, Case)> = cases_in("cases")
        .into_iter()
        .map(|(stem, case)| (format!("case/{stem}"), case))
        .collect();
    let seeds = if dir.join("seeds").is_dir() {
        cases_in("seeds")
    } else {
        Vec::new()
    };
    for (stem, case) in &seeds {
        projects.push((format!("seed/{stem}"), case.clone()));
    }
    let edits = edit::parse_table(&fs::read_to_string(dir.join("mutations.tsv")).unwrap()).unwrap();
    for edit in edits {
        let (_, seed) = seeds.iter().find(|(stem, _)| *stem == edit.seed).unwrap();
        projects.push((format!("mutation/{}", edit.id), edit.apply(seed)));
    }
    projects
}

/// What elm said and what nomi said about one project as one release.
struct Run {
    /// `<project id>/<release>`, the ratchet's case.
    id: String,
    elm_json: String,
    elm_terminal: String,
    json: String,
    terminal: String,
    /// The `docs.json` each wrote, for `--docs` projects.
    elm_docs: Option<String>,
    docs: Option<String>,
}

/// Every project of the suite in `dir` as every release, built as `goal`
/// asks, but where elm crashed; each run as it comes.
fn runs(dir: &Path, goal: Goal) -> impl Iterator<Item = Run> {
    let written = match goal {
        Goal::Docs => table(&dir.join("docs.tsv")),
        Goal::Check | Goal::Optimize => Default::default(),
    };
    let reports = table(&dir.join("reports.tsv"));
    let terminal = table(&dir.join("terminal.tsv"));
    let default = fs::read_to_string(dir.join("default.elm.json")).unwrap();
    let cases = projects(dir)
        .into_iter()
        .flat_map(|(id, case)| Compat::ALL.map(|compat| (id.clone(), case.clone(), compat)));
    cases.filter_map(move |(id, case, compat)| {
        let key = (id, compat_name(compat));
        let elm_json = reports[&key].clone();
        if elm_json == CRASH {
            return None;
        }
        let reported = projects::run(&case, compat, &default, goal);
        Some(Run {
            id: format!("{}/{compat}", key.0),
            elm_docs: (written.get(&key)).filter(|cell| *cell != "none").cloned(),
            elm_json,
            elm_terminal: serde_json::from_str(&terminal[&key]).unwrap(),
            json: reported.json,
            terminal: reported.terminal,
            docs: reported.docs,
        })
    })
}
