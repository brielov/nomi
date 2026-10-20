//! nomi/core's `Nomi.program`, bundled with nomi: built offline from the
//! bundle, it is a Bun entry unless the browser is asked for, and runs
//! its task as the process, which exits with how the task ended, or at
//! once, naming it, when its host isn't Bun 1.4 or later; joining a fiber
//! that dies is the joiner's defect, and its fibers share refs and queues
//! (`nomi_test.rs` runs its concurrent tasks, which take hours, on
//! nomi/test's virtual clock); `Nomi.log` writes to stderr. `nomi_http.rs`
//! tests nomi/http, bundled beside it.
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
/// with a finalizer, when they are `wait`; logs two lines, one from a
/// fiber, when they are `log`; and otherwise prints them and the variable
/// `GREETING`.
const PROGRAM: &str = r#"module Main exposing (main)

import Dict
import Nomi
import Nomi.Fiber as Fiber
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

                [ "log" ] ->
                    Nomi.log "first"
                        |> Task.andThen (\_ -> Fiber.fork (Nomi.log "from a fiber"))
                        |> Task.andThen Fiber.join

                _ ->
                    say (Debug.toString ( env.args, Dict.get "GREETING" env.vars ))


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

/// A program that joins a fiber that dies.
const DEFECT: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Fiber as Fiber
import Task


main : Nomi.Program
main =
    Nomi.program <|
        \_ ->
            Fiber.fork (Task.succeed () |> Task.andThen (\_ -> Debug.todo "boom"))
                |> Task.andThen Fiber.join
                |> Task.map (\_ -> Debug.log "unreachable" ())
"#;

/// A program that shares a `Ref` and `Queue`s between fibers, printing
/// what each step gives. Its waits are short, and what it prints doesn't
/// depend on how long they take.
const SHARED: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Fiber as Fiber
import Nomi.Queue as Queue exposing (Queue)
import Nomi.Ref as Ref
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \_ -> Task.sequence checks |> Task.map (\_ -> ())


checks : List (Task String ())
checks =
    [ Ref.make 0
        |> Task.andThen
            (\ref ->
                Fiber.fork (after 1 (Ref.modify (\n -> ( n, n + 1 )) ref))
                    |> List.repeat 100
                    |> Task.sequence
                    |> Task.andThen (Task.sequence << List.map Fiber.join)
                    |> Task.andThen (\_ -> Ref.get ref)
            )
        |> show "modify"
    , Ref.make 1
        |> Task.andThen (\ref -> Ref.set 5 ref |> Task.andThen (\_ -> Ref.update ((*) 2) ref) |> Task.andThen (\_ -> Ref.get ref))
        |> show "set and update"
    , Queue.bounded 2
        |> Task.andThen
            (\queue ->
                List.range 1 5
                    |> List.map (\n -> Queue.offer n queue |> Task.andThen (\_ -> say ("offered " ++ String.fromInt n)))
                    |> Task.sequence
                    |> Fiber.fork
                    |> Task.andThen (\_ -> Task.sequence (List.repeat 5 (after 10 (Queue.take queue |> Task.andThen (\n -> say ("took " ++ String.fromInt n))))))
            )
        |> show "bounded"
    , Queue.unbounded |> Task.andThen Queue.poll |> show "poll"
    , Queue.unbounded
        |> Task.andThen (\queue -> Queue.offer "a" queue |> Task.andThen (\_ -> Queue.poll queue))
        |> show "poll"
    , Queue.unbounded
        |> Task.andThen
            (\queue ->
                [ "a", "b", "c" ]
                    |> List.map (\name -> Fiber.fork (Queue.take queue |> Task.andThen (\n -> say (name ++ " took " ++ String.fromInt n))))
                    |> Task.sequence
                    |> Task.andThen (\fibers -> after 10 (offerAll [ 1, 2, 3 ] queue) |> Task.andThen (\_ -> Task.sequence (List.map Fiber.join fibers)))
            )
        |> show "takers in order"
    , Queue.unbounded
        |> Task.andThen
            (\queue ->
                Fiber.fork (Queue.take queue |> Task.andThen (\_ -> say "unreachable"))
                    |> Task.andThen (\taker -> after 10 (Fiber.interrupt taker))
                    |> Task.andThen (\_ -> offerAll [ 7 ] queue)
                    |> Task.andThen (\_ -> Queue.poll queue)
            )
        |> show "interrupted take"
    , Queue.bounded 1
        |> Task.andThen
            (\queue ->
                offerAll [ 1 ] queue
                    |> Task.andThen (\_ -> Fiber.fork (offerAll [ 2 ] queue))
                    |> Task.andThen (\offerer -> after 10 (Fiber.interrupt offerer))
                    |> Task.andThen (\_ -> Task.map2 Tuple.pair (Queue.take queue) (Queue.poll queue))
            )
        |> show "interrupted offer"
    ]


offerAll : List a -> Queue a -> Task x ()
offerAll items queue =
    List.map (\item -> Queue.offer item queue) items |> Task.sequence |> Task.map (\_ -> ())


after : Float -> Task x a -> Task x a
after ms task =
    Process.sleep ms |> Task.andThen (\_ -> task)


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

/// An application of `program` on nomi/core and nomi/http, in a new
/// directory.
fn project(program: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/http": "2.0.0"}, "indirect": {"elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
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

fn bun() -> Command {
    Command::new(nomi_testkit::bun())
}

/// The lines of standard error past the development mode's warning.
fn errors(output: &Output) -> Vec<String> {
    (String::from_utf8_lossy(&output.stderr).lines())
        .filter(|line| !line.starts_with("Compiled in DEV mode."))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_program_is_a_bun_entry_by_default_and_its_task_decides_the_exit() {
    nomi_testkit::require_bun();
    let (_work, root) = built(PROGRAM);
    let ran = bun()
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
    let failed = bun()
        .args(["dist/main.mjs", "fail"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert_eq!(errors(&failed), ["nomi: the program failed: x"]);
}

#[test]
fn nomi_log_writes_a_line_to_stderr() {
    nomi_testkit::require_bun();
    let (_work, root) = built(PROGRAM);
    let ran = bun()
        .args(["dist/main.mjs", "log"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "");
    assert_eq!(errors(&ran), ["first", "from a fiber"]);
}

#[test]
fn sigterm_interrupts_a_program_runs_its_finalizer_and_exits_143() {
    nomi_testkit::require_bun();
    let (_work, root) = built(PROGRAM);
    let mut program = bun()
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

/// Runs the entry `dist/main.mjs` in a fresh global scope whose host is
/// `host`, a JS expression: its `Bun`, if any, and its `process`. The
/// entry's probe runs before anything else of it, so a refused host
/// exits there, which the scope's `process.exit` reports as a line.
const ON_HOST: &str = r#"
const vm = require("node:vm");
const fs = require("node:fs");
const [file, host] = process.argv.slice(1);
const code = fs.readFileSync(file, "utf8").replace(/\nexport const Elm = scope\.Elm;\n$/, "\n");
const exited = new Error("exited");
const scope = { console, ...eval(`(${host})`) };
scope.process.exit = (code) => { console.log(`exit ${code}`); throw exited; };
try {
  vm.runInNewContext(code, scope);
  console.log("ran");
} catch (error) {
  if (error !== exited) throw error;
}
"#;

#[test]
fn an_entry_on_a_host_that_isnt_a_recent_bun_exits_saying_so() {
    nomi_testkit::require_bun();
    let (_work, root) = built(PROGRAM);
    let hosts = [
        (
            r#"{ process: { version: "v24.21.0", release: { name: "node" } } }"#,
            "nomi: this program runs on Bun 1.4 or later (this is Node v24.21.0).",
        ),
        (
            r#"{ Bun: { version: "1.3.9" }, process: {} }"#,
            "nomi: this program runs on Bun 1.4 or later (this is Bun 1.3.9).",
        ),
    ];
    for (host, said) in hosts {
        let ran = bun()
            .args(["-e", ON_HOST, "dist/main.mjs", host])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
        assert_eq!(String::from_utf8_lossy(&ran.stdout), "exit 1\n", "{ran:?}");
        assert_eq!(errors(&ran), [said]);
    }
}

#[test]
fn joining_a_fiber_that_dies_is_the_joiners_defect() {
    nomi_testkit::require_bun();
    let (_work, root) = built(DEFECT);
    let ran = bun()
        .arg("dist/main.mjs")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "");
    // The runtime formats the report itself, so it reads the same on any
    // host: the name and message, then only the stack's frames.
    let errors = errors(&ran);
    let (frames, report): (Vec<_>, Vec<_>) =
        errors.iter().partition(|line| line.starts_with("    at "));
    assert_eq!(
        report,
        [
            "nomi: the program died with a defect: Error: TODO in module `Main` on line 12",
            "",
            "boom",
        ],
        "{errors:?}"
    );
    assert!(!frames.is_empty(), "{errors:?}");
    assert!(
        errors[report.len()..]
            .iter()
            .all(|line| line.starts_with("    at ")),
        "{errors:?}"
    );
}

#[test]
fn fibers_share_refs_and_queues_in_order_and_interruption_loses_no_item() {
    nomi_testkit::require_bun();
    let (_work, root) = built(SHARED);
    let ran = bun()
        .arg("dist/main.mjs")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(errors(&ran), Vec::<String>::new());
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout)
            .lines()
            .collect::<Vec<_>>(),
        [
            "modify: Ok 100",
            "set and update: Ok 10",
            "offered 1: ()",
            "offered 2: ()",
            "took 1: ()",
            "offered 3: ()",
            "took 2: ()",
            "offered 4: ()",
            "took 3: ()",
            "offered 5: ()",
            "took 4: ()",
            "took 5: ()",
            "bounded: Ok [(),(),(),(),()]",
            "poll: Ok Nothing",
            "poll: Ok (Just \"a\")",
            "a took 1: ()",
            "b took 2: ()",
            "c took 3: ()",
            "takers in order: Ok [(),(),()]",
            "interrupted take: Ok (Just 7)",
            "interrupted offer: Ok (1,Nothing)",
        ]
    );
}
