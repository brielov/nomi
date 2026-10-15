//! nomi/core's `Nomi.program`, bundled with nomi: built offline from the
//! bundle, it is a Node entry unless the browser is asked for, and runs
//! its task as the process, which exits with how the task ended.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use website::offline;

/// A program that fails with `x` when its arguments are `fail`; waits,
/// with a finalizer, when they are `wait`; and otherwise prints them and
/// the variable `GREETING`.
const PROGRAM: &str = r#"module Main exposing (main)

import Dict
import Nomi
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "fail" ] ->
                    Task.fail "x"

                [ "wait" ] ->
                    say "waiting"
                        |> Task.andThen (\_ -> Process.sleep 60000)
                        |> Nomi.Task.ensure (say "finalized")

                _ ->
                    say (Debug.toString ( env.args, Dict.get "GREETING" env.vars ))


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

/// An application of [`PROGRAM`] on nomi/core, in a new directory.
fn project() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), PROGRAM).unwrap();
    (work, root)
}

/// `nomi build src/Main.elm` with `args` in `root`, offline with `cache`.
fn build(cache: &Path, root: &Path, args: &[&str]) -> Output {
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let mut build = offline(program, cache);
    build
        .current_dir(root)
        .args(["build", "src/Main.elm"])
        .args(args);
    build.output().unwrap()
}

/// [`PROGRAM`] built with no `--platform` into `dist/main.mjs`, from an
/// empty store and offline, so from the bundle.
fn built() -> (tempfile::TempDir, PathBuf) {
    let cache = website::fixture_cache();
    let (work, root) = project();
    let built = build(cache.path(), &root, &["--output=dist/main.mjs"]);
    assert!(built.status.success(), "{built:?}");
    assert!(!cache.path().join("packages/releases/nomi").exists());
    (work, root)
}

/// Node's own binary: a version manager's `node` may exit 128 plus the
/// signal for a child the signal killed.
fn node() -> Command {
    let path = Command::new("node")
        .args(["-p", "process.execPath"])
        .output()
        .unwrap();
    Command::new(String::from_utf8(path.stdout).unwrap().trim())
}

/// The lines of standard error past the development mode's warning.
fn errors(output: &Output) -> Vec<String> {
    (String::from_utf8_lossy(&output.stderr).lines())
        .filter(|line| !line.starts_with("Compiled in DEV mode."))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_program_is_a_node_entry_by_default_and_its_task_decides_the_exit() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    let ran = node()
        .args(["dist/main.mjs", "a", "b"])
        .env("GREETING", "hi")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "([\"a\",\"b\"],Just \"hi\"): ()\n"
    );
    assert_eq!(errors(&ran), Vec::<String>::new());
    let failed = node()
        .args(["dist/main.mjs", "fail"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert_eq!(errors(&failed), ["nomi: the program failed: x"]);
}

#[test]
fn sigterm_interrupts_a_program_runs_its_finalizer_and_exits_143() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    let mut program = node()
        .args(["dist/main.mjs", "wait"])
        .current_dir(&root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(program.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "waiting: ()\n");
    let sent = Command::new("kill")
        .args(["-TERM".to_owned(), program.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let signalled = Instant::now();
    while program.try_wait().unwrap().is_none() && signalled.elapsed().as_secs() < 15 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = program.kill();
    let ran = program.wait_with_output().unwrap();
    assert_eq!(ran.status.code(), Some(143), "{ran:?}");
    let mut rest = String::new();
    stdout.read_line(&mut rest).unwrap();
    assert_eq!(rest, "finalized: ()\n");
    assert_eq!(errors(&ran), Vec::<String>::new());
}

#[test]
fn a_program_built_for_the_browser_names_the_path_to_the_nomi_kernel() {
    let cache = website::fixture_cache();
    let (_work, root) = project();
    let built = build(
        cache.path(),
        &root,
        &["--platform=browser", "--output=dist/main.js"],
    );
    assert!(!built.status.success(), "{built:?}");
    let said = String::from_utf8_lossy(&built.stdout) + String::from_utf8_lossy(&built.stderr);
    assert!(said.contains("NOT AVAILABLE IN THE BROWSER"), "{said}");
    assert!(
        said.contains(
            "The `main` in Main uses Elm.Kernel.Nomi, which the browser does not have:\n\n    \
             main → Nomi.Task.ensure → Elm.Kernel.Nomi\n"
        ),
        "{said}"
    );
    assert!(!root.join("dist").exists());
}
