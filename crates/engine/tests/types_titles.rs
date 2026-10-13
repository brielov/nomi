//! The first type ratchet: each type-error project as each release,
//! passing when nomi reports the problems elm does, by title and region,
//! in each module elm names.
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

#[path = "support/names.rs"]
mod names;

use nomi_testkit::ratchet::Suite;
use serde_json::Value;

/// A report reduced to the project's problem's title, or each module's
/// path and its problems' titles and regions.
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

#[test]
fn types_titles() {
    let dir = names::suite("types");
    let outcomes = names::runs_in(&dir).into_iter().map(|run| {
        let (elm, nomi) = (titles(&run.elm_json), titles(&run.json));
        let outcome = if elm == nomi {
            Ok(())
        } else {
            Err(format!("nomi: {nomi}\nelm: {elm}"))
        };
        if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
            eprintln!("FAIL {}\n{why}", run.id);
        }
        (run.id, outcome)
    });
    Suite::open(dir.with_file_name("types_titles")).finish(outcomes);
}
