//! The second naming ratchet: each naming project as each release,
//! passing when nomi's `--report=json` and terminal report are elm's,
//! whole (plan 5.4; climbs in 5.14).
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

#[path = "support/names.rs"]
mod names;

use nomi_testkit::ratchet::Suite;

#[test]
fn names_reports() {
    let outcomes = names::runs().into_iter().map(|run| {
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
    Suite::open(names::dir().with_file_name("names_reports")).finish(outcomes);
}
