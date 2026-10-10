//! The documentation ratchet: each documentation project as each
//! release, passing when nomi's `--report=json`, terminal report and
//! `docs.json` are elm's, whole (plan 7.4).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

#[path = "support/names.rs"]
mod names;

use nomi_testkit::ratchet::Suite;

#[test]
fn docs_reports() {
    let dir = names::suite("docs");
    let outcomes = names::docs_runs_in(&dir).into_iter().map(|run| {
        let outcome = if !projects::same_json(&run.elm_json, &run.json) {
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
        if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
            eprintln!("FAIL {}\n{why}", run.id);
        }
        (run.id, outcome)
    });
    Suite::open(dir.with_file_name("docs_reports")).finish(outcomes);
}
