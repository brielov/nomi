//! The pattern ratchet: each pattern-error project as each release,
//! passing when nomi's `--report=json` and terminal report are elm's,
//! whole (plan 7.1).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

#[path = "support/names.rs"]
mod names;

use nomi_testkit::ratchet::Suite;

#[test]
fn patterns_reports() {
    let dir = names::suite("patterns");
    let outcomes = names::runs_in(&dir).into_iter().map(|run| {
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
        } else {
            Ok(())
        };
        if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
            eprintln!("FAIL {}\n{why}", run.id);
        }
        (run.id, outcome)
    });
    Suite::open(dir.with_file_name("patterns_reports")).finish(outcomes);
}
