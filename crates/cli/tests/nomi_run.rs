//! `nomi run`: an entry of nomi.toml, or a module given with `-e`, built
//! for Bun and run under the `bun` on the PATH with the arguments after
//! `--`; nomi exits as the program does, passes SIGTERM on to it, and
//! runs nothing when the build or nomi.toml is wrong. With `--watch` it
//! builds and runs the program again as the project changes.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nomi_testkit::http::{events, request};
use nomi_testkit::process::{Group, Signal, expect, free_port, next_lines};
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
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::write(dir.join("src/Main.elm"), program).unwrap();
    if let Some(nomi_toml) = nomi_toml {
        fs::write(dir.join("nomi.toml"), nomi_toml).unwrap();
    }
}

/// `nomi run` with `args` in `root`, offline with `cache`, with the tests'
/// Bun binary's directory first on the PATH.
fn nomi_run(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let mut run = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    run.current_dir(root)
        .env("PATH", nomi_testkit::path_with_bun())
        .arg("run")
        .args(args);
    run
}

/// `nomi run` with `args` in `root`, as [`nomi_run`] gives it, started in
/// a group of its own with its streams piped.
fn spawn_in(cache: &Path, root: &Path, args: &[&str]) -> Group {
    let mut run = nomi_run(cache, root, args);
    Group::spawn(run.stdout(Stdio::piped()).stderr(Stdio::piped()))
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
    nomi_testkit::require_bun();
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
fn a_chosen_entry_writes_its_output() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let nomi_toml = "[[entry]]\nmain = \"src/Other.elm\"\nplatform = \"browser\"\n\n\
                     [[entry]]\nmain = \"src/Main.elm\"\noutput = \"dist/main.mjs\"\n";
    let (_work, root) = project(PROGRAM, Some(nomi_toml));
    let unchosen = nomi_run(cache.path(), &root, &["--", "3"])
        .output()
        .unwrap();
    assert_eq!(unchosen.status.code(), Some(1), "{unchosen:?}");
    assert!(
        stderr(&unchosen).contains("-- ARGUMENTS FOR WHICH ENTRY?"),
        "{unchosen:?}"
    );
    // From another directory, by its path from there.
    let ran = nomi_run(cache.path(), &root.join("src"), &["./Main.elm", "--", "4"])
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(4), "{ran:?}");
    assert_eq!(stdout(&ran), "4: ()\n");
    assert!(root.join("dist/main.mjs").is_file());
}

#[test]
fn sigterm_and_sigint_reach_the_program_which_finalizes_and_nomi_exits_as_it_does() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    for (signal, code) in [(Signal::TERM, 143), (Signal::INT, 130)] {
        let mut run = spawn_in(cache.path(), &root, &["--", "wait"]);
        let stdout = run.stdout();
        assert_eq!(next_lines(&stdout, 1), ["waiting: ()"]);
        run.signal(signal);
        assert_eq!(run.ended(), Some(code), "{signal:?}");
        // Printed by the program after the signal: it was passed on, and
        // the program, not nomi, ended the run.
        assert_eq!(next_lines(&stdout, 1), ["finalized: ()"], "{signal:?}");
    }
}

#[test]
fn a_module_given_with_e_runs_without_nomi_toml() {
    nomi_testkit::require_bun();
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
I know `main`, `output`, `platform`, `args`, `static` and `port` here.

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
fn without_bun_on_the_path_nomi_says_so() {
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    let ran = nomi_run(cache.path(), &root, &["--", "3"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert!(stderr(&ran).contains("-- NO BUN"), "{ran:?}");
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
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = project(VERSIONED, Some(ENTRY));
    version(&root, "\"1\"");
    let mut run = spawn_in(cache.path(), &root, &["--watch"]);
    let (stdout, stderr) = (run.stdout(), run.stderr());
    let mut said = expect(&stderr, "Starting src/Main.elm");
    assert_eq!(expect(&stdout, "version 1: ()"), [] as [String; 0]);
    // The old program is stopped as SIGTERM stops it, its finalizer
    // running, before the new one starts.
    version(&root, "\"22\"");
    said.extend(expect(&stderr, "src/Version.elm changed"));
    said.extend(expect(&stderr, "Restarting src/Main.elm"));
    assert_eq!(expect(&stdout, "version 22: ()"), ["finalized 1: ()"]);
    // A build that fails stops the program and waits for the next change.
    version(&root, "333");
    said.extend(expect(&stderr, "-- TYPE MISMATCH"));
    said.extend(expect(
        &stderr,
        "Stopping src/Main.elm, as its build failed",
    ));
    said.extend(expect(&stderr, "Watching for changes..."));
    assert_eq!(expect(&stdout, "finalized 22: ()"), [] as [String; 0]);
    version(&root, "\"4444\"");
    expect(&stdout, "version 4444: ()");
    // SIGTERM stops the program and the watch.
    run.signal(Signal::TERM);
    expect(&stdout, "finalized 4444: ()");
    assert_eq!(run.ended(), Some(143));
    // Nor the first start nor a restart repeats DEV mode's advice.
    said.extend(stderr.iter());
    assert!(
        !said
            .iter()
            .any(|line| line.contains("Compiled in DEV mode")),
        "{said:?}"
    );
}

#[test]
fn with_watch_a_program_that_ends_is_started_again_on_a_change() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = project(PROGRAM, Some(ENTRY));
    let mut run = spawn_in(cache.path(), &root, &["--watch", "--", "3"]);
    let (stdout, stderr) = (run.stdout(), run.stderr());
    expect(&stdout, "3: ()");
    expect(&stderr, "src/Main.elm exited with code 3");
    fs::write(
        root.join("src/Main.elm"),
        PROGRAM.replace("say code", "say (code ++ \"!\")"),
    )
    .unwrap();
    expect(&stdout, "3!: ()");
}

/// A program for the browser, by its `main`'s type.
const WORKER: &str = "module Main exposing (main)\n\n\nmain : Program () () ()\nmain =\n    \
                      Platform.worker\n        { init = \\_ -> ( (), Cmd.none )\n        \
                      , update = \\_ model -> ( model, Cmd.none )\n        \
                      , subscriptions = \\_ -> Sub.none\n        }\n";

#[test]
fn a_browser_entry_is_served_until_nomi_is_stopped() {
    let cache = website::fixture_cache();
    let port = free_port();
    let nomi_toml = format!("{ENTRY}port = {port}\n");
    let (_work, root) = project(WORKER, Some(&nomi_toml));
    let mut run = spawn_in(cache.path(), &root, &[]);
    let stderr = run.stderr();
    let serving = format!("Serving src/Main.elm at http://localhost:{port}");
    expect(&stderr, &serving);
    // Built where its main's type says, without a platform: a script for
    // the browser, in a page titled by its project.
    let page = request(port, "GET", "/").unwrap();
    assert_eq!(page.status, 200);
    assert!(
        page.text().contains("<script src=\"/Main.js\"></script>"),
        "{page:?}"
    );
    let title = root.file_name().unwrap().to_string_lossy();
    assert!(
        page.text().contains(&format!("<title>{title}</title>")),
        "{page:?}"
    );
    // Nothing reloads it without --watch.
    assert!(!page.text().contains("/__nomi/events"), "{page:?}");
    assert_eq!(request(port, "GET", "/__nomi/events").unwrap().status, 404);
    let code = request(port, "GET", "/Main.js").unwrap();
    assert!(!code.text().contains("export const Elm"), "{code:?}");
    assert!(!code.text().contains("Compiled in DEV mode"), "{code:?}");
    assert!(root.join("elm-stuff/nomi/run/src/Main.js").is_file());
    assert!(!root.join("elm-stuff/nomi/run/src/Main.mjs").exists());
    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
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

/// The next line of `lines` with `wanted` in it, failing after a minute.
fn line_with(lines: &Receiver<String>, wanted: &str) -> String {
    loop {
        match lines.recv_timeout(Duration::from_secs(60)) {
            Ok(line) if line.contains(wanted) => return line,
            Ok(_) => {}
            Err(e) => panic!("no line with {wanted:?}: {e}"),
        }
    }
}

/// The port the line of `lines` saying `name` is served gives.
fn served_at(lines: &[String], name: &str) -> u16 {
    let serving = format!("Serving {name} at http://localhost:");
    let at = (lines.iter())
        .find_map(|line| line.split_once(&serving))
        .unwrap_or_else(|| panic!("{name} isn't served: {lines:?}"))
        .1;
    let digits: String = at.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap()
}

#[test]
fn a_taken_port_assigned_moves_to_the_next_free_one() {
    let cache = website::fixture_cache();
    // Held here once another test's server lets it go, or held all along
    // by something else.
    let start = Instant::now();
    let _taken = loop {
        match TcpListener::bind("127.0.0.1:8000") {
            Ok(taken) => break Some(taken),
            Err(_) if start.elapsed() > Duration::from_secs(60) => break None,
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let (_work, root) = project(WORKER, Some(ENTRY));
    let mut run = spawn_in(cache.path(), &root, &[]);
    let stderr = run.stderr();
    let line = line_with(&stderr, "Serving src/Main.elm");
    assert!(line.ends_with(", as port 8000 is taken"), "{line}");
    let port = served_at(&[line], "src/Main.elm");
    assert_ne!(port, 8000);
    assert_eq!(request(port, "GET", "/").unwrap().status, 200);
    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
}

/// A program that says it is waiting, then waits with a finalizer; or,
/// given `exit` and a code, waits until a file `go` appears in the
/// directory it runs in, then exits with that code; given `slow`, serves
/// with a minute's drain and a finalizer that sleeps that minute, so a
/// signal stops it no sooner.
const GATED: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Process
import Nomi.Server as Server
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
                        |> Task.andThen (\_ -> gate 6000 env.fs)
                        |> Task.andThen (\_ -> Nomi.Process.exit (Maybe.withDefault 1 (String.toInt code)))

                [ "slow" ] ->
                    let
                        config =
                            Server.defaults
                    in
                    Server.listen
                        { config | port_ = 0, drainMilliseconds = 60000, onListen = \_ -> say "waiting" }
                        (\_ -> Task.succeed (Server.empty 204))
                        env.net
                        |> Task.mapError Server.errorToString
                        |> Nomi.Task.ensure (Process.sleep 60000)

                _ ->
                    say "waiting"
                        |> Task.andThen (\_ -> Process.sleep 60000)
                        |> Nomi.Task.ensure (say "finalized")


{-| Waits until `go` exists, looking every 10 ms, and gives up after
`tries` looks, so a test that fails before writing it leaves nothing
waiting.
-}
gate : Int -> Fs -> Task String ()
gate tries fs =
    Fs.exists "go" fs
        |> Task.andThen
            (\go ->
                if go then
                    Task.succeed ()

                else if tries <= 0 then
                    Task.fail "no go within a minute"

                else
                    Process.sleep 10 |> Task.andThen (\_ -> gate (tries - 1) fs)
            )


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

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
        let entry = format!("[[entry]]\nmain = \"src/Main.elm\"\nargs = {args}\n");
        app(&root.join(dir), GATED, Some(&entry));
        fs::write(
            root.join(dir).join("elm.json"),
            r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/server": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
        )
        .unwrap();
    }
    (work, root)
}

#[test]
fn every_entry_runs_under_its_member_and_sigterm_finalizes_them_all() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let mut run = spawn_in(cache.path(), &root, &[]);
    let stdout = run.stdout();
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    run.signal(Signal::TERM);
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    finalized: ()", "[apps/worker] finalized: ()"]
    );
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn at_the_root_browser_entries_are_served_beside_the_programs() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let port = free_port();
    let web = root.join("apps/web");
    let nomi_toml = format!("{ENTRY}static = \"public\"\nport = {port}\n");
    app(&web, WORKER, Some(&nomi_toml));
    fs::create_dir_all(web.join("public")).unwrap();
    fs::write(web.join("public/index.html"), "<p>web</p>\n").unwrap();
    let mut run = spawn_in(cache.path(), &root, &[]);
    let (stdout, stderr) = (run.stdout(), run.stderr());
    let serving = format!("[nomi] Serving apps/web at http://localhost:{port}");
    expect(&stderr, &serving);
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    let page = request(port, "GET", "/a/route").unwrap();
    assert_eq!(page.text(), "<p>web</p>\n");
    run.signal(Signal::TERM);
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    finalized: ()", "[apps/worker] finalized: ()"]
    );
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn an_entry_that_fails_stops_the_others_and_gives_nomi_its_code() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"exit\", \"3\"]", "[\"wait\"]");
    let mut run = spawn_in(cache.path(), &root, &[]);
    let stdout = run.stdout();
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    fs::write(root.join("go"), "").unwrap();
    expect(&stdout, "[apps/worker] finalized: ()");
    assert_eq!(run.ended(), Some(3));
}

#[cfg(unix)]
#[test]
fn nomi_exits_once_its_programs_have_though_a_process_they_started_holds_their_output() {
    use std::os::unix::fs::PermissionsExt as _;
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (work, root) = gated("[\"helper\"]", "[\"wait\"]");
    // A `bun` that, for api, first starts a helper on its standard
    // streams, as a program starting a process of its own would.
    let bin = work.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::write(
        bin.join("bun"),
        format!(
            "#!/bin/sh\ncase \"$*\" in *helper*) (echo helper; exec sleep 300) & ;; esac\nexec '{}' \"$@\"\n",
            nomi_testkit::bun().display()
        ),
    )
    .unwrap();
    fs::set_permissions(bin.join("bun"), fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(nomi_testkit::path_with_bun());
    let mut run = nomi_run(cache.path(), &root, &[]);
    run.env("PATH", path);
    let mut run = Group::spawn(run.stdout(Stdio::piped()).stderr(Stdio::piped()));
    let stdout = run.stdout();
    expect(&stdout, "[apps/api]    helper");
    run.signal(Signal::TERM);
    // The helper runs on for minutes, holding api's pipes open.
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn a_program_that_does_not_stop_within_the_grace_is_killed() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"slow\"]");
    let mut run = spawn_in(cache.path(), &root, &[]);
    let (stdout, stderr) = (run.stdout(), run.stderr());
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    run.signal(Signal::TERM);
    expect(&stdout, "[apps/api]    finalized: ()");
    expect(
        &stderr,
        "[apps/worker] The program did not stop within 15 seconds, so I killed it.",
    );
    assert_eq!(run.ended(), Some(143));
}

/// The processes whose parent is `parent`, by their ids.
#[cfg(target_os = "linux")]
fn children(parent: u32) -> Vec<u32> {
    let parent = parent.to_string();
    (fs::read_dir("/proc").unwrap().flatten())
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            // `pid (comm) state ppid ...`, the name in parentheses.
            let fields = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
            fields.split_whitespace().nth(1) == Some(parent.as_str())
        })
        .collect()
}

/// Whether the process `pid` has ended, reaped or not.
#[cfg(target_os = "linux")]
fn has_ended(pid: u32) -> bool {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    !matches!(state.split_whitespace().next(), Some(s) if s != "Z" && s != "X")
}

#[cfg(target_os = "linux")]
#[test]
fn programs_end_when_nomi_is_killed() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"wait\"]");
    let mut run = spawn_in(cache.path(), &root, &[]);
    let stdout = run.stdout();
    assert_eq!(
        next_lines(&stdout, 2),
        ["[apps/api]    waiting: ()", "[apps/worker] waiting: ()"]
    );
    let programs = children(run.id());
    assert_eq!(programs.len(), 2, "{programs:?}");
    run.signal(Signal::KILL);
    // Each is sent SIGTERM, where else it would wait a minute.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !programs.iter().all(|&pid| has_ended(pid)) {
        assert!(Instant::now() < deadline, "{programs:?} still run");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `nomi run` with `args` in `root`, as [`nomi_run`] gives it, on a
/// terminal of its own that `script` makes: what is written to `script`
/// is typed there, and what nomi and its programs print, both streams,
/// is its output.
#[cfg(target_os = "linux")]
fn on_a_terminal(cache: &Path, root: &Path) -> (Group, std::process::ChildStdin, Receiver<String>) {
    let run = nomi_run(cache, root, &[]);
    let line = std::iter::once(run.get_program())
        .chain(run.get_args())
        .map(|word| format!("'{}'", word.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = Command::new("script");
    script
        .args(["--quiet", "--return", "--command", &line, "/dev/null"])
        .current_dir(root)
        .env("SHELL", "/bin/sh")
        .envs(run.get_envs().filter_map(|(k, v)| Some((k, v?))))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut group = Group::spawn(&mut script);
    let typed = group.child().stdin.take().unwrap();
    let said = group.stdout();
    (group, typed, said)
}

#[cfg(target_os = "linux")]
#[test]
fn a_terminal_interrupt_reaches_the_programs_itself_and_a_second_kills_them() {
    use std::io::Write as _;
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = gated("[\"wait\"]", "[\"slow\"]");
    let (mut run, mut typed, said) = on_a_terminal(cache.path(), &root);
    // Both programs' lines, in either order.
    expect(&said, "waiting: ()");
    expect(&said, "waiting: ()");
    // The terminal sends both programs SIGINT itself, and nomi passes
    // nothing on, so each sees one: api finalizes, and the worker starts
    // its finalizer, which holds it.
    typed.write_all(b"\x03").unwrap();
    expect(&said, "finalized: ()");
    // A second kills the worker at once, without waiting out the grace.
    typed.write_all(b"\x03").unwrap();
    assert_eq!(run.ended(), Some(130));
    let rest: Vec<String> = said.iter().collect();
    assert!(
        !rest.iter().any(|line| line.contains("did not stop")),
        "{rest:?}"
    );
}

/// Waits for `events`, a page's event stream, to say reload, failing
/// after a minute.
fn reloaded(events: &Receiver<String>) {
    expect(events, "event: reload");
}

#[test]
fn with_watch_a_browser_entry_is_served_and_reloaded_as_it_changes() {
    let cache = website::fixture_cache();
    let port = free_port();
    let nomi_toml = format!("{ENTRY}static = \"public\"\nport = {port}\n");
    let (_work, root) = project(WORKER, Some(&nomi_toml));
    fs::create_dir_all(root.join("public")).unwrap();
    let mut run = spawn_in(cache.path(), &root, &["--watch"]);
    let stderr = run.stderr();
    expect(&stderr, "Watching for changes...");
    let (status, events) = events(port, "/__nomi/events").unwrap();
    assert_eq!(status, 200);
    let edited = WORKER.replace(
        "\\_ -> ( (), Cmd.none )",
        "\\_ -> ( always () \"edited\", Cmd.none )",
    );
    fs::write(root.join("src/Main.elm"), edited).unwrap();
    expect(&stderr, "Reloading src/Main.elm");
    reloaded(&events);
    let code = request(port, "GET", "/Main.js").unwrap();
    assert!(code.text().contains("edited"), "{code:?}");
    // A static file reloads the page without a build.
    let output = root.join("elm-stuff/nomi/run/src/Main.js");
    let built = fs::metadata(&output).unwrap().modified().unwrap();
    fs::write(root.join("public/app.css"), "p {}\n").unwrap();
    expect(&stderr, "Reloading src/Main.elm");
    reloaded(&events);
    assert_eq!(fs::metadata(&output).unwrap().modified().unwrap(), built);
    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn with_watch_a_page_whose_first_build_failed_is_served_once_fixed() {
    let cache = website::fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("nomi.toml"),
        "[workspace]\nmembers = [\"apps/*\"]\n",
    )
    .unwrap();
    let broken = WORKER.replace("( (), Cmd.none )", "( 1, Cmd.none )");
    app(&root.join("apps/w1"), &broken, Some(ENTRY));
    app(&root.join("apps/w2"), WORKER, Some(ENTRY));
    let mut run = spawn_in(cache.path(), &root, &["--watch"]);
    let stderr = run.stderr();
    let started = expect(&stderr, "[nomi] Watching for changes...");
    assert!(
        started.iter().any(|line| line.contains("-- TYPE MISMATCH")),
        "{started:?}"
    );
    let w2 = served_at(&started, "apps/w2");
    fs::write(root.join("apps/w1/src/Main.elm"), WORKER).unwrap();
    let fixed = expect(&stderr, "[nomi] Watching for changes...");
    let w1 = served_at(&fixed, "apps/w1");
    assert_ne!(w1, w2);
    for port in [w1, w2] {
        assert_eq!(request(port, "GET", "/").unwrap().status, 200);
    }
    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
}

/// A workspace of `packages/shared`, the package acme/shared exposing
/// `Version`; `apps/api`, a Bun program of [`VERSIONED`]; and `apps/web`,
/// a browser program whose code holds the version, served on `port`
/// with static files.
fn full_stack(port: u16) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let files = [
        ("nomi.toml", "[workspace]\nmembers = [\"apps/*\", \"packages/*\"]\n".to_owned()),
        (
            "packages/shared/elm.json",
            r#"{"type": "package", "name": "acme/shared", "summary": "shared", "license": "BSD-3-Clause", "version": "1.0.0", "exposed-modules": ["Version"], "elm-version": "0.19.0 <= v < 0.20.0", "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"}, "test-dependencies": {}}"#.to_owned(),
        ),
        (
            "apps/api/elm.json",
            r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"acme/shared": "1.0.0", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#.to_owned(),
        ),
        ("apps/api/nomi.toml", ENTRY.to_owned()),
        ("apps/api/src/Main.elm", VERSIONED.to_owned()),
        (
            "apps/web/elm.json",
            r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"acme/shared": "1.0.0", "elm/core": "1.0.5", "elm/json": "1.1.4"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#.to_owned(),
        ),
        (
            "apps/web/nomi.toml",
            format!("{ENTRY}static = \"public\"\nport = {port}\n"),
        ),
        ("apps/web/src/Main.elm", web("")),
        ("apps/web/public/index.html", "<p>web</p>\n".to_owned()),
        ("packages/shared/src/Version.elm", String::new()),
    ];
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    shared(&root, "\"1\"");
    (work, root)
}

/// The web application, starting from the length of the version and
/// `more`.
fn web(more: &str) -> String {
    WORKER
        .replace(
            "module Main exposing (main)\n",
            "module Main exposing (main)\n\nimport Version\n",
        )
        .replace("Program () () ()", "Program () Int ()")
        .replace(
            "\\_ -> ( (), Cmd.none )",
            &format!("\\_ -> ( String.length (Version.version ++ \"{more}\"), Cmd.none )"),
        )
}

/// The package's `Version` module, its version `version`.
fn shared(root: &Path, version: &str) {
    fs::write(
        root.join("packages/shared/src/Version.elm"),
        format!(
            "module Version exposing (version)\n\n\nversion : String\nversion =\n    {version}\n"
        ),
    )
    .unwrap();
}

/// What nomi says on standard error for the next change, up to its
/// watching again.
fn batch(stderr: &Receiver<String>) -> String {
    expect(stderr, "[nomi] Watching for changes...").join("\n")
}

#[test]
fn with_watch_a_workspace_restarts_and_reloads_only_what_a_change_changes() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let port = free_port();
    let (_work, root) = full_stack(port);
    let mut run = spawn_in(cache.path(), &root, &["--watch"]);
    let (stdout, stderr) = (run.stdout(), run.stderr());
    let started = batch(&stderr);
    assert!(
        started.contains(&format!(
            "[nomi] Serving apps/web at http://localhost:{port}"
        )),
        "{started}"
    );
    expect(&stdout, "[apps/api] version 1: ()");
    let (_, events) = events(port, "/__nomi/events").unwrap();

    // A module of web's alone reloads web, and api runs on.
    fs::write(root.join("apps/web/src/Main.elm"), web("!")).unwrap();
    let web_edit = batch(&stderr);
    assert!(web_edit.contains("[nomi] Reloading apps/web"), "{web_edit}");
    assert!(!web_edit.contains("[nomi] Restarting"), "{web_edit}");
    reloaded(&events);

    // A static file reloads web without a build.
    fs::write(root.join("apps/web/public/app.css"), "p {}\n").unwrap();
    let static_edit = batch(&stderr);
    assert!(
        static_edit.contains("[nomi] Reloading apps/web"),
        "{static_edit}"
    );
    assert!(!static_edit.contains("Building"), "{static_edit}");
    reloaded(&events);

    // The package's body restarts api and reloads web.
    shared(&root, "\"22\"");
    let body_edit = batch(&stderr);
    assert!(
        body_edit.contains("[nomi] Restarting apps/api"),
        "{body_edit}"
    );
    assert!(
        body_edit.contains("[nomi] Reloading apps/web"),
        "{body_edit}"
    );
    assert_eq!(
        expect(&stdout, "[apps/api] version 22: ()"),
        ["[apps/api] finalized 1: ()"]
    );
    reloaded(&events);

    // A comment changes no code, so nothing restarts or reloads.
    shared(&root, "\"22\" -- the version");
    let comment = batch(&stderr);
    assert!(
        comment.contains("[nomi] packages/shared/src/Version.elm changed"),
        "{comment}"
    );
    for said in ["Restarting", "Starting", "Stopping", "Reloading"] {
        assert!(!comment.contains(said), "{comment}");
    }

    // A type error in the package stops api and leaves web's page as it is.
    shared(&root, "333");
    let broken = batch(&stderr);
    assert!(broken.contains("-- TYPE MISMATCH"), "{broken}");
    assert!(
        broken.contains("[nomi] Stopping apps/api, as its build failed"),
        "{broken}"
    );
    assert!(!broken.contains("Reloading"), "{broken}");
    expect(&stdout, "[apps/api] finalized 22: ()");
    let page = request(port, "GET", "/").unwrap();
    assert_eq!(page.status, 200);

    // Its fix starts api again and reloads web.
    shared(&root, "\"4444\"");
    let fixed = batch(&stderr);
    assert!(fixed.contains("[nomi] Starting apps/api"), "{fixed}");
    assert!(fixed.contains("[nomi] Reloading apps/web"), "{fixed}");
    expect(&stdout, "[apps/api] version 4444: ()");
    reloaded(&events);

    run.signal(Signal::TERM);
    expect(&stdout, "[apps/api] finalized 4444: ()");
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn with_watch_a_page_moves_as_its_entry_does() {
    let cache = website::fixture_cache();
    let (old, new) = (free_port(), free_port());
    let (_work, root) = project(WORKER, Some(&format!("{ENTRY}port = {old}\n")));
    let mut run = spawn_in(cache.path(), &root, &["--watch"]);
    let stderr = run.stderr();
    expect(
        &stderr,
        &format!("Serving src/Main.elm at http://localhost:{old}"),
    );
    let moved = format!("{ENTRY}port = {new}\noutput = \"dist/app.js\"\n");
    fs::write(root.join("nomi.toml"), moved).unwrap();
    expect(
        &stderr,
        &format!("Serving src/Main.elm at http://localhost:{new}"),
    );
    let page = request(new, "GET", "/").unwrap();
    assert!(
        page.text().contains("<script src=\"/app.js\"></script>"),
        "{page:?}"
    );
    assert_eq!(request(new, "GET", "/app.js").unwrap().status, 200);
    assert!(request(old, "GET", "/").is_err());
    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
}

#[test]
fn with_watch_one_member_restarts_as_the_member_packages_it_uses_change() {
    nomi_testkit::require_bun();
    let cache = website::fixture_cache();
    let (_work, root) = full_stack(free_port());
    let api = root.join("apps/api");
    for (dir, args) in [
        (&api, &["--watch"][..]),
        (&root, &["--watch", "-p", "apps/api"]),
    ] {
        shared(&root, "\"1\"");
        let mut run = spawn_in(cache.path(), dir, args);
        let (stdout, stderr) = (run.stdout(), run.stderr());
        expect(&stdout, "version 1: ()");
        shared(&root, "\"22\"");
        let restarted = expect(&stderr, "Restarting src/Main.elm");
        assert!(
            restarted
                .iter()
                .any(|line| line.contains("Version.elm changed")),
            "{args:?}: {restarted:?}"
        );
        assert_eq!(expect(&stdout, "version 22: ()"), ["finalized 1: ()"]);
        run.signal(Signal::TERM);
        assert_eq!(run.ended(), Some(143), "{args:?}");
    }
}
