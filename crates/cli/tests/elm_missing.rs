//! The commands nomi leaves out as `elm`: `elm repl` and `elm
//! reactor` exit with one line saying so, the reactor's pointing to
//! `nomi run --watch`.
//!
//! One test, so the binary is copied once: a copy made while another test
//! starts a process can be held open in that process, and fail to run.

#[path = "support/website.rs"]
pub mod website;

use std::path::Path;
use std::process::Command;

/// What `elm` says on standard error given `args`, which it must refuse
/// with nothing on standard output.
fn refusal(elm: &Path, args: &[&str]) -> String {
    let output = Command::new(elm).args(args).output().unwrap();
    assert!(!output.status.success() && output.stdout.is_empty());
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn repl_and_reactor_explain_themselves() {
    let (_bin, elm) = website::personality("elm");
    assert_eq!(
        refusal(&elm, &["repl", "--no-colors"]),
        "error: nomi has no REPL, so `elm repl` isn't here; elm's own still works beside it\n"
    );
    assert_eq!(
        refusal(&elm, &["reactor", "--port=8000"]),
        "error: nomi has no reactor; `nomi run --watch` rebuilds and serves your entries as you edit\n"
    );
}
