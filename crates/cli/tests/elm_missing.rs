//! The commands nomi leaves out as `elm` (plan 8.15): `elm repl` and `elm
//! reactor` exit with one line saying so, the reactor's pointing to
//! `nomi run --watch`.
//!
//! One test, so the binary is copied once: a copy made while another test
//! starts a process can be held open in that process, and fail to run.

use std::fs;
use std::path::Path;
use std::process::Command;

fn elm(elm: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(elm).args(args).output().unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn repl_and_reactor_explain_themselves() {
    let bin = tempfile::tempdir().unwrap();
    let path = bin
        .path()
        .join(format!("elm{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &path).unwrap();

    let (ok, stdout, stderr) = elm(&path, &["repl", "--no-colors"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error: nomi has no REPL, so `elm repl` isn't here; elm's own still works beside it\n"
    );

    let (ok, stdout, stderr) = elm(&path, &["reactor", "--port=8000"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error: nomi has no reactor; `nomi run --watch` rebuilds and serves your entries as you edit\n"
    );
}
