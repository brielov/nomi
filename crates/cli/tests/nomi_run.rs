//! `nomi run`: an entry of nomi.toml, or a module given with `-e`, built
//! for Node and run under the `node` on the PATH with the arguments after
//! `--`; nomi exits as the program does, passes SIGTERM on to it, and
//! runs nothing when the build or nomi.toml is wrong. With `--watch` it
//! builds and runs the program again as the project changes.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nomi_testkit::http::request;
use nomi_testkit::process::{expect, lines};
use website::offline;

/// A program that exits with the code it is given, after printing it;
/// with `wait`, it waits with a finalizer.
const PROGRAM: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Process
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "wait" ] ->
                    say "waiting"
                        |> Task.andThen (\_ -> Process.sleep 60000)
                        |> Nomi.Task.ensure (say "finalized")

                [ code ] ->
                    case String.toInt code of
                        Just n ->
                            say code |> Task.andThen (\_ -> Nomi.Process.exit n)

                        Nothing ->
                            Task.fail "not a code"

                _ ->
                    say "no code"


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

/// An application of `program` on nomi/core, with `nomi_toml` if any,
/// in a new directory.
fn project(program: &str, nomi_toml: Option<&str>) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    app(&root, program, nomi_toml);
    (work, root)
}

/// An application of `program` on nomi/core in `dir`, with `nomi_toml`
/// if any.
fn app(dir: &Path, program: &str, nomi_toml: Option<&str>) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::write(dir.join("src/Main.elm"), program).unwrap();
    if let Some(nomi_toml) = nomi_toml {
        fs::write(dir.join("nomi.toml"), nomi_toml).unwrap();
    }
}

/// `nomi run` with `args` in `root`, offline with `cache`, with the tests'
/// Node binary's directory first on the PATH.
fn nomi_run(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let node = nomi_testkit::node();
    let mut path = OsString::from(node.parent().unwrap());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let mut run = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    run.current_dir(root)
        .env("PATH", path)
        .arg("run")
        .args(args);
    run
}

const ENTRY: &str = "[[entry]]\nmain = \"src/Main.elm\"\n";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn an_entry_runs_with_its_arguments_and_exits_with_its_code() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(3), "{ran:?}");
    // The build speaks on standard error; standard output is the program's.
    assert_eq!(stdout(&ran), "3: ()\n");
    assert!(root.join("elm-stuff/nomi/run/src/Main.mjs").is_file());
}

#[test]
fn a_chosen_entry_writes_its_output_and_imports_its_ffi_module() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let program = PROGRAM
        .replace("import Nomi\n", "import Json.Decode\nimport Json.Encode\nimport Nomi\nimport Nomi.Ffi\n")
        .replace(
            "say \"no code\"",
            "Nomi.Ffi.task \"twice\" (Json.Encode.int 21) Json.Decode.int |> Task.mapError (\\_ -> \"ffi\") |> Task.andThen (\\n -> say (String.fromInt n))",
        );
    let nomi_toml = "[[entry]]\nmain = \"src/Other.elm\"\nplatform = \"browser\"\n\n\
                     [[entry]]\nmain = \"src/Main.elm\"\noutput = \"dist/main.mjs\"\nffi = \"src/ffi.mjs\"\n";
    let (_work, root) = project(&program, Some(nomi_toml));
    fs::write(
        root.join("src/ffi.mjs"),
        "export async function twice(n) { return n * 2; }\n",
    )
    .unwrap();
    let unchosen = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(unchosen.status.code(), Some(1), "{unchosen:?}");
    assert!(
        stderr(&unchosen).contains("-- ARGUMENTS FOR WHICH ENTRY?"),
        "{unchosen:?}"
    );
    // From another directory, by its path from there.
    let ran = nomi_run(cache.path(), &root.join("src"), &["./Main.elm"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(stdout(&ran), "42: ()\n");
    assert!(root.join("dist/main.mjs").is_file());
}

#[test]
fn sigterm_and_sigint_reach_the_program_which_finalizes_and_nomi_exits_as_it_does() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    for (signal, code) in [("-TERM", 143), ("-INT", 130)] {
        let mut run = nomi_run(cache.path(), &root, &["--", "wait"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(run.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert_eq!(line, "waiting: ()\n");
        let sent = Command::new("kill")
            .args([signal.to_owned(), run.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
        let signalled = Instant::now();
        while run.try_wait().unwrap().is_none() && signalled.elapsed().as_secs() < 15 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = run.kill();
        let ran = run.wait_with_output().unwrap();
        assert_eq!(ran.status.code(), Some(code), "{signal}: {ran:?}");
        // Printed by the program after the signal: it was passed on, and
        // the program, not nomi, ended the run.
        let mut rest = String::new();
        stdout.read_line(&mut rest).unwrap();
        assert_eq!(rest, "finalized: ()\n", "{signal}");
    }
}

#[test]
fn a_module_given_with_e_runs_without_nomi_toml() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, None);
    let ran = nomi_run(cache.path(), &root, &["-e", "src/Main.elm", "--", "0"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(stdout(&ran), "0: ()\n");
    let unnamed = nomi_run(cache.path(), &root, &[]).output().unwrap();
    assert_eq!(unnamed.status.code(), Some(1), "{unnamed:?}");
    assert!(stderr(&unnamed).contains("-- NO NOMI.TOML"), "{unnamed:?}");
}

#[test]
fn a_bad_nomi_toml_is_shown_at_its_span_and_nothing_runs() {
    let cache = website::fixture_cache();
    let (_work, root) = project(
        PROGRAM,
        Some("[[entry]]\nmain = \"src/Main.elm\"\nplatfrom = \"node\"\n"),
    );
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(stdout(&ran), "");
    assert_eq!(
        stderr(&ran),
        "-- UNKNOWN KEY ------------------------------------------------------- nomi.toml

I ran into a problem with your nomi.toml file. I do not know the `platfrom` key
here:

3| platfrom = \"node\"
   ^^^^^^^^
I know `main`, `output`, `platform`, `ffi`, `args`, `static` and `port` here.

"
    );
}

#[test]
fn a_build_that_fails_runs_nothing() {
    let cache = website::fixture_cache();
    let (_work, root) = project(&PROGRAM.replace("say code", "say n"), Some(ENTRY));
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(stdout(&ran), "");
    assert!(stderr(&ran).contains("-- TYPE MISMATCH"), "{ran:?}");
    assert!(!root.join("elm-stuff/nomi/run/src/Main.mjs").exists());
}

#[test]
fn without_node_on_the_path_nomi_says_so() {
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert!(stderr(&ran).contains("-- NO NODE"), "{ran:?}");
}

/// A program that says which version of `Version` it was built with,
/// then waits with a finalizer saying it again.
const VERSIONED: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Task
import Process
import Task exposing (Task)
import Version


main : Nomi.Program
main =
    Nomi.program <|
        \_ ->
            say ("version " ++ Version.version)
                |> Task.andThen (\_ -> Process.sleep 60000)
                |> Nomi.Task.ensure (say ("finalized " ++ Version.version))


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

fn version(root: &Path, version: &str) {
    fs::write(
        root.join("src/Version.elm"),
        format!("module Version exposing (version)\n\n\nversion =\n    {version}\n"),
    )
    .unwrap();
}

#[test]
fn with_watch_an_edit_restarts_the_program_with_its_new_output() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(VERSIONED, Some(ENTRY));
    version(&root, "\"1\"");
    let mut run = nomi_run(cache.path(), &root, &["--watch"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = lines(run.stdout.take().unwrap());
    let stderr = lines(run.stderr.take().unwrap());
    assert_eq!(
        expect(&stdout, "version 1: ()", &mut run),
        [] as [String; 0]
    );
    // The old program is stopped as SIGTERM stops it, its finalizer
    // running, before the new one starts.
    version(&root, "\"22\"");
    expect(&stderr, "src/Version.elm changed", &mut run);
    assert_eq!(
        expect(&stdout, "version 22: ()", &mut run),
        ["finalized 1: ()"]
    );
    // A build that fails stops the program and waits for the next change.
    version(&root, "333");
    expect(&stderr, "-- TYPE MISMATCH", &mut run);
    expect(&stderr, "Watching for changes...", &mut run);
    assert_eq!(
        expect(&stdout, "finalized 22: ()", &mut run),
        [] as [String; 0]
    );
    version(&root, "\"4444\"");
    expect(&stdout, "version 4444: ()", &mut run);
    // SIGTERM stops the program and the watch.
    let sent = Command::new("kill")
        .args(["-TERM".to_owned(), run.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    expect(&stdout, "finalized 4444: ()", &mut run);
    let signalled = Instant::now();
    while run.try_wait().unwrap().is_none() && signalled.elapsed().as_secs() < 30 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = run.kill();
    assert_eq!(run.wait().unwrap().code(), Some(143));
}

#[test]
fn with_watch_a_program_that_ends_is_started_again_on_a_change() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    let mut run = nomi_run(cache.path(), &root, &["--watch", "--", "3"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = lines(run.stdout.take().unwrap());
    let stderr = lines(run.stderr.take().unwrap());
    expect(&stdout, "3: ()", &mut run);
    expect(
        &stderr,
        "The program exited with code 3. Watching for changes...",
        &mut run,
    );
    fs::write(
        root.join("src/Main.elm"),
        PROGRAM.replace("say code", "say (code ++ \"!\")"),
    )
    .unwrap();
    expect(&stdout, "3!: ()", &mut run);
    let _ = run.kill();
    let _ = run.wait();
}

/// A program for the browser, by its `main`'s type.
const WORKER: &str = "module Main exposing (main)\n\n\nmain : Program () () ()\nmain =\n    \
                      Platform.worker\n        { init = \\_ -> ( (), Cmd.none )\n        \
                      , update = \\_ model -> ( model, Cmd.none )\n        \
                      , subscriptions = \\_ -> Sub.none\n        }\n";

/// A port nothing listens on just now.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Sends `signal` to `child`.
fn signal(child: &Child, signal: &str) {
    let sent = Command::new("kill")
        .args([signal.to_owned(), child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
}

#[test]
fn a_browser_entry_is_served_until_nomi_is_stopped() {
    let cache = website::fixture_cache();
    let port = free_port();
    let nomi_toml = format!("{ENTRY}port = {port}\n");
    let (_work, root) = project(WORKER, Some(&nomi_toml));
    let mut run = nomi_run(cache.path(), &root, &[])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = lines(run.stderr.take().unwrap());
    let serving = format!("Serving src/Main.elm at http://localhost:{port}");
    expect(&stderr, &serving, &mut run);
    // Built where its main's type says, as an ES module, for want of a
    // platform before the build.
    let page = request(port, "GET", "/").unwrap();
    assert_eq!(page.status, 200);
    assert!(
        page.text().contains("import { Elm } from \"/Main.mjs\";"),
        "{page:?}"
    );
    let code = request(port, "GET", "/Main.mjs").unwrap();
    assert!(code.text().contains("export const Elm"), "{code:?}");
    signal(&run, "-TERM");
    assert_eq!(ended(&mut run), Some(143));
}

#[test]
fn a_taken_port_serves_nothing() {
    let cache = website::fixture_cache();
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let nomi_toml = format!("{ENTRY}platform = \"browser\"\nport = {port}\n");
    let (_work, root) = project(WORKER, Some(&nomi_toml));
    let ran = nomi_run(cache.path(), &root, &[]).output().unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    let shown = stderr(&ran);
    assert!(shown.contains("-- PORT TAKEN"), "{ran:?}");
    assert!(
        shown.contains(&format!("src/Main.elm is served on port {port}")),
        "{ran:?}"
    );
    assert!(root.join("elm-stuff/nomi/run/src/Main.js").is_file());
}

/// A program that says it is waiting, then waits with a finalizer; or,
/// given `exit` and a code, waits until a file `go` appears in the
/// directory it runs in, then exits with that code.
const GATED: &str = r#"module Main exposing (main)

import Json.Decode
import Json.Encode
import Nomi
import Nomi.Ffi
import Nomi.Process
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "exit", code ] ->
                    say "waiting"
                        |> Task.andThen (\_ -> Nomi.Ffi.task "gate" Json.Encode.null (Json.Decode.succeed ()))
                        |> Task.mapError (\_ -> "no gate")
                        |> Task.andThen (\_ -> Nomi.Process.exit (Maybe.withDefault 1 (String.toInt code)))

                _ ->
                    say "waiting"
                        |> Task.andThen (\_ -> Process.sleep 60000)
                        |> Nomi.Task.ensure (say "finalized")


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

const GATE: &str = "import { existsSync } from \"node:fs\";\n\n\
                    export async function gate() {\n  \
                    while (!existsSync(\"go\")) await new Promise((r) => setTimeout(r, 10));\n}\n";

/// A workspace of two applications of [`GATED`], `apps/api` and
/// `apps/worker`, whose entries take `api` and `worker` as their `args`.
fn gated(api: &str, worker: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("nomi.toml"),
        "[workspace]\nmembers = [\"apps/*\"]\n",
    )
    .unwrap();
    for (dir, args) in [("apps/api", api), ("apps/worker", worker)] {
        let entry =
            format!("[[entry]]\nmain = \"src/Main.elm\"\nffi = \"src/ffi.mjs\"\nargs = {args}\n");
        app(&root.join(dir), GATED, Some(&entry));
        fs::write(root.join(dir).join("src/ffi.mjs"), GATE).unwrap();
    }
    (work, root)
}

/// `nomi run` at the root of `root`, its streams piped.
fn spawn_at_root(cache: &Path, root: &Path) -> Child {
    nomi_run(cache, root, &[])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// How `run` ended, killing it if it hasn't within 30 seconds.
fn ended(run: &mut Child) -> Option<i32> {
    let start = Instant::now();
    while run.try_wait().unwrap().is_none() && start.elapsed().as_secs() < 30 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = run.kill();
    run.wait().unwrap().code()
}

#[test]
fn every_entry_runs_under_its_member_and_sigterm_finalizes_them_all() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let mut run = spawn_at_root(cache.path(), &root);
    let stdout = lines(run.stdout.take().unwrap());
    assert_eq!(
        two_lines(&stdout, &mut run),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    let sent = Command::new("kill")
        .args(["-TERM".to_owned(), run.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    assert_eq!(
        two_lines(&stdout, &mut run),
        ["[apps/api]    finalized: ()", "[apps/worker] finalized: ()"]
    );
    assert_eq!(ended(&mut run), Some(143));
}

#[test]
fn at_the_root_browser_entries_are_served_beside_the_programs() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let port = free_port();
    let web = root.join("apps/web");
    let nomi_toml = format!("{ENTRY}static = \"public\"\nport = {port}\n");
    app(&web, WORKER, Some(&nomi_toml));
    fs::create_dir_all(web.join("public")).unwrap();
    fs::write(web.join("public/index.html"), "<p>web</p>\n").unwrap();
    let mut run = spawn_at_root(cache.path(), &root);
    let stdout = lines(run.stdout.take().unwrap());
    let stderr = lines(run.stderr.take().unwrap());
    let serving = format!("[apps/web]    Serving src/Main.elm at http://localhost:{port}");
    expect(&stderr, &serving, &mut run);
    assert_eq!(
        two_lines(&stdout, &mut run),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    let page = request(port, "GET", "/a/route").unwrap();
    assert!(page.text().starts_with("<p>web</p>\n<script>"), "{page:?}");
    signal(&run, "-TERM");
    assert_eq!(
        two_lines(&stdout, &mut run),
        ["[apps/api]    finalized: ()", "[apps/worker] finalized: ()"]
    );
    assert_eq!(ended(&mut run), Some(143));
}

/// The next two lines of `lines`, which come in either order, sorted;
/// killing `child` and failing after a minute.
fn two_lines(lines: &Receiver<String>, child: &mut Child) -> Vec<String> {
    let mut two: Vec<String> = (0..2)
        .map(|_| {
            (lines.recv_timeout(Duration::from_secs(60))).unwrap_or_else(|e| {
                let _ = child.kill();
                panic!("no line: {e}")
            })
        })
        .collect();
    two.sort();
    two
}

#[test]
fn an_entry_that_fails_stops_the_others_and_gives_nomi_its_code() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"exit\", \"3\"]", "[\"wait\"]");
    let mut run = spawn_at_root(cache.path(), &root);
    let stdout = lines(run.stdout.take().unwrap());
    assert_eq!(
        two_lines(&stdout, &mut run),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    fs::write(root.join("go"), "").unwrap();
    expect(&stdout, "[apps/worker] finalized: ()", &mut run);
    assert_eq!(ended(&mut run), Some(3));
}

#[test]
fn arguments_go_to_one_entry() {
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert!(
        stderr(&ran).contains("-- ARGUMENTS FOR WHICH ENTRY?"),
        "{ran:?}"
    );
    assert!(!root.join("apps/api/elm-stuff").exists());
}
