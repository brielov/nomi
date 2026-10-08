//! `nomi format` as a user runs it.

use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};

const FORMATTED: &str = "module A exposing (x)\n\n\nx =\n    1\n";
const UNFORMATTED: &str = "module A exposing (x)\nx  =  1\n";
const INVALID: &str = "module A exposing (x)\nx = (\n";

fn nomi(args: &[&str], dir: &std::path::Path, stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nomi"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap()
}

#[test]
fn formats_the_current_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("A.elm"), UNFORMATTED).unwrap();
    let output = nomi(&["format"], dir.path(), "");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(output.stdout), "");
    assert_eq!(
        fs::read_to_string(dir.path().join("A.elm")).unwrap(),
        FORMATTED
    );
}

#[test]
fn check_lists_unformatted_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("A.elm"), UNFORMATTED).unwrap();
    fs::write(dir.path().join("B.elm"), FORMATTED).unwrap();
    let output = nomi(&["fmt", "--check", "A.elm", "B.elm"], dir.path(), "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(output.stdout), "A.elm\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("A.elm")).unwrap(),
        UNFORMATTED
    );

    let output = nomi(&["fmt", "--check", "B.elm"], dir.path(), "");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn errors_name_the_place() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("A.elm"), INVALID).unwrap();
    fs::create_dir(dir.path().join("empty")).unwrap();
    let output = nomi(&["format", "A.elm", "empty"], dir.path(), "");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(output.stderr),
        "error: empty: no .elm files\nerror: A.elm:3:1: syntax error\n"
    );
}

#[test]
fn stdin_goes_to_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let output = nomi(&["format", "--stdin"], dir.path(), UNFORMATTED);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(text(output.stdout), FORMATTED);

    let output = nomi(&["format", "--stdin", "--check"], dir.path(), UNFORMATTED);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(output.stdout), "<stdin>\n");

    let output = nomi(&["format", "--stdin"], dir.path(), INVALID);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(output.stdout), "");
    assert_eq!(text(output.stderr), "error: <stdin>:3:1: syntax error\n");
}

#[test]
fn stdin_takes_no_paths() {
    let dir = tempfile::tempdir().unwrap();
    let output = nomi(&["format", "--stdin", "A.elm"], dir.path(), "");
    assert_eq!(output.status.code(), Some(2));
}
