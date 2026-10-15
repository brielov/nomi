//! nomi/core's `Nomi.program`, bundled with nomi: built offline from the
//! bundle, it is a Node entry unless the browser is asked for, and runs
//! its task as the process, which exits with how the task ended; its
//! concurrent tasks and fibers end and interrupt as documented.
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

/// A program that runs nomi/core's concurrent tasks and fibers, printing
/// how each ends; with the argument `defect`, it joins a fiber that dies.
/// What it waits for to lose takes an hour, so it ends at once only on the
/// virtual clock.
const CONCURRENT: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Fiber as Fiber exposing (Exit(..))
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "defect" ] ->
                    Fiber.fork (Task.succeed () |> Task.andThen (\_ -> Debug.todo "boom"))
                        |> Task.andThen Fiber.join
                        |> Task.andThen (\_ -> say "unreachable")

                _ ->
                    Task.sequence checks |> Task.map (\_ -> ())


checks : List (Task String ())
checks =
    [ show "both" (Nomi.Task.both (after 10 (Task.fail "first")) (hour "both's other"))
    , show "both" (Nomi.Task.both (after 20 (Task.succeed 1)) (after 10 (Task.succeed 2)))
    , show "all" (Nomi.Task.all [ after 30 (Task.succeed 1), after 10 (Task.succeed 2), after 20 (Task.succeed 3) ])
    , show "race" (Nomi.Task.race (after 10 (Task.succeed "fast")) (hour "race's loser"))
    , show "timeout" (Nomi.Task.timeout 50 (hour "timed out"))
    , show "timeout" (Nomi.Task.timeout 50 (after 10 (Task.succeed "quick")))
    , Fiber.fork
        (say "region starts"
            |> Task.andThen (\_ -> Process.sleep 100)
            |> Task.andThen (\_ -> say "region ends")
            |> Nomi.Task.uninterruptible
        )
        |> Task.andThen (interruptAfter 50)
        |> show "uninterruptible"
    , Fiber.fork (Task.succeed 1) |> Task.andThen Fiber.await |> show "await"
    , Fiber.fork (Task.fail "no") |> Task.andThen Fiber.await |> show "await"
    , Fiber.fork (Task.succeed () |> Task.andThen (\_ -> Debug.todo "boom"))
        |> Task.andThen Fiber.await
        |> Task.map defect
        |> show "await"
    , Fiber.fork (hour "awaited") |> Task.andThen (interruptAfter 10) |> show "await"
    , Fiber.fork (Fiber.forkDaemon (after 10 (Task.succeed "daemon")))
        |> Task.andThen Fiber.join
        |> Task.andThen Fiber.await
        |> show "forkDaemon"
    , Fiber.fork (Fiber.fork (after 10 (Task.succeed "child")))
        |> Task.andThen Fiber.join
        |> Task.andThen Fiber.await
        |> show "fork"
    , show "acquireRelease" (resource "a" (Task.succeed 1))
    , show "acquireRelease" (resource "b" (Task.fail "no"))
    , Fiber.fork (resource "c" (hour "using c")) |> Task.andThen (interruptAfter 10) |> show "acquireRelease"
    ]


after : Float -> Task x a -> Task x a
after ms task =
    Process.sleep ms |> Task.andThen (\_ -> task)


hour : String -> Task String a
hour name =
    after 3600000 (Task.fail "too late") |> Nomi.Task.ensure (say (name ++ " finalized"))


interruptAfter : Float -> Fiber.Fiber x a -> Task y (Exit x a)
interruptAfter ms fiber =
    Process.sleep ms
        |> Task.andThen (\_ -> say "interrupting")
        |> Task.andThen (\_ -> Fiber.interrupt fiber)


resource : String -> Task String a -> Task String a
resource name use =
    Nomi.Task.acquireRelease
        (say ("acquired " ++ name) |> Task.map (\_ -> name))
        (\r -> say ("released " ++ r))
        (\_ -> use)


defect : Exit x a -> String
defect exit =
    case exit of
        Defect message ->
            "Defect, " ++ String.left 36 message

        _ ->
            "not a defect"


show : String -> Task x a -> Task y ()
show label task =
    task
        |> Task.map Ok
        |> Task.onError (Task.succeed << Err)
        |> Task.map (\result -> Debug.log label result |> always ())


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

/// An application of `program` on nomi/core, in a new directory.
fn project(program: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), program).unwrap();
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

/// `program` built with no `--platform` into `dist/main.mjs`, from an
/// empty store and offline, so from the bundle.
fn built(program: &str) -> (tempfile::TempDir, PathBuf) {
    let cache = website::fixture_cache();
    let (work, root) = project(program);
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
    let (_work, root) = built(PROGRAM);
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
    let (_work, root) = built(PROGRAM);
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
    let (_work, root) = project(PROGRAM);
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

#[test]
fn concurrent_tasks_and_fibers_end_and_interrupt_as_documented_on_the_virtual_clock() {
    nomi_testkit::require_node();
    let (_work, root) = built(CONCURRENT);
    let started = Instant::now();
    let ran = node()
        .arg("dist/main.mjs")
        .env("NOMI_CLOCK", "virtual")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(errors(&ran), Vec::<String>::new());
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout)
            .lines()
            .collect::<Vec<_>>(),
        [
            "both's other finalized: ()",
            "both: Err \"first\"",
            "both: Ok (1,2)",
            "all: Ok [1,2,3]",
            "race's loser finalized: ()",
            "race: Ok \"fast\"",
            "timed out finalized: ()",
            "timeout: Ok Nothing",
            "timeout: Ok (Just \"quick\")",
            "region starts: ()",
            "interrupting: ()",
            "region ends: ()",
            "uninterruptible: Ok Interrupted",
            "await: Ok (Success 1)",
            "await: Ok (Failure \"no\")",
            "await: Ok \"Defect, Error: TODO in module `Main` on line\"",
            "interrupting: ()",
            "awaited finalized: ()",
            "await: Ok Interrupted",
            "forkDaemon: Ok (Success \"daemon\")",
            "fork: Ok Interrupted",
            "acquired a: ()",
            "released a: ()",
            "acquireRelease: Ok 1",
            "acquired b: ()",
            "released b: ()",
            "acquireRelease: Err \"no\"",
            "acquired c: ()",
            "interrupting: ()",
            "using c finalized: ()",
            "released c: ()",
            "acquireRelease: Ok Interrupted",
        ]
    );
}

#[test]
fn joining_a_fiber_that_dies_is_the_joiners_defect() {
    nomi_testkit::require_node();
    let (_work, root) = built(CONCURRENT);
    let ran = node()
        .args(["dist/main.mjs", "defect"])
        .env("NOMI_CLOCK", "virtual")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "");
    let errors = errors(&ran);
    assert!(
        errors[0].starts_with("nomi: the program died with a defect: Error: TODO in module `Main`"),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|line| line.contains("boom")),
        "{errors:?}"
    );
    assert!(
        !errors.iter().any(|line| line.starts_with("nomi: fiber")),
        "{errors:?}"
    );
}
