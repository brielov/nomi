//! The first naming ratchet: each naming project as each release, passing
//! when nomi reports the problems elm does, by title and region, in each
//! module elm names (plan 5.4; climbs in 5.5 to 5.13).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

#[path = "support/names.rs"]
mod names;

use nomi_testkit::ratchet::Suite;
use serde_json::Value;

/// A report reduced to what this ratchet holds nomi to: the project's
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
        let problems = module["problems"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| serde_json::json!([p["title"], p["region"]]));
        serde_json::json!([module["path"], problems.collect::<Vec<_>>()])
    });
    Value::Array(modules.collect())
}

#[test]
fn names_titles() {
    let outcomes = names::runs().into_iter().map(|run| {
        let (elm, nomi) = (titles(&run.elm_json), titles(&run.json));
        let outcome = if elm == nomi {
            Ok(())
        } else {
            Err(format!("nomi: {nomi}\nelm: {elm}"))
        };
        (run.id, outcome)
    });
    Suite::open(names::dir().with_file_name("names_titles")).finish(outcomes);
}
