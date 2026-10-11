//! The commands nomi leaves out as `elm` (plan 8.15): `elm repl` and `elm
//! reactor` exit with one line saying so, the reactor's pointing to
//! `nomi run --watch`.

use std::fs;
use std::process::Command;

fn elm(args: &[&str]) -> (bool, String, String) {
    let bin = tempfile::tempdir().unwrap();
    let elm = bin
        .path()
        .join(format!("elm{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm).unwrap();
    let output = Command::new(&elm).args(args).output().unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn repl_explains_itself() {
    let (ok, stdout, stderr) = elm(&["repl", "--no-colors"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error: nomi has no REPL, so `elm repl` isn't here; elm's own still works beside it\n"
    );
}

#[test]
fn reactor_points_to_run_watch() {
    let (ok, stdout, stderr) = elm(&["reactor", "--port=8000"]);
    assert!(!ok);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "error: nomi has no reactor; `nomi run --watch` rebuilds and serves your entries as you edit\n"
    );
}
