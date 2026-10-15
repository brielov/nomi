//! `nomi build`: each case under `nomi_build/cases/` must
//! reproduce the transcript of the same name under `expected/`. nomi's
//! own messages have no oracle, so the transcripts are snapshots, read
//! when written: `NOMI_BLESS=1` writes them anew.
//!
//! Cases run offline, with a cache serving the packages vendored for the
//! compiler's tests.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nomi_testkit::transcript::check_transcripts;
use website::offline;

#[test]
fn nomi_build_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_build");
    let cache = website::fixture_cache();
    let bless = std::env::var_os("NOMI_BLESS").is_some();
    check_transcripts(&dir.join("cases"), &dir.join("expected"), |name, case| {
        let work = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(work.path()).unwrap();
        case.create_files(&root).unwrap();
        let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let actual = case
            .run(offline(program, cache.path()), &root)
            .unwrap()
            .compared(case, &root);
        if bless {
            fs::write(dir.join("expected").join(format!("{name}.txt")), &actual).unwrap();
        }
        Some(actual)
    });
}

/// A project built once keeps its dependencies as solved until its
/// elm.json changes, as elm keeps `elm-stuff/d.dat`: the next build
/// reads no package list.
#[test]
fn a_built_project_keeps_its_dependencies_solved() {
    let cache = website::fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let outline = |core: &str| {
        format!(
            r#"{{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {{"direct": {{"elm/core": "{core}", "elm/json": "1.1.4"}}, "indirect": {{}}}}, "test-dependencies": {{"direct": {{}}, "indirect": {{}}}}}}"#
        )
    };
    fs::write(root.join("elm.json"), outline("1.0.5")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/Main.elm"),
        "module Main exposing (value)\n\nvalue =\n    1\n",
    )
    .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let build = || {
        let mut command = offline(program, cache.path());
        command.current_dir(&root).args(["check", "src/Main.elm"]);
        command.output().unwrap()
    };
    let first = build();
    assert!(first.status.success(), "{first:?}");
    let index = cache.path().join("registry/index");
    fs::remove_file(&index).unwrap();
    let second = build();
    assert!(second.status.success(), "{second:?}");
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&second.stdout)
    );
    // Changed, it is solved again, and without the package list that fails.
    fs::write(root.join("elm.json"), outline("1.0.4")).unwrap();
    let third = build();
    let said = String::from_utf8_lossy(&third.stdout) + String::from_utf8_lossy(&third.stderr);
    assert!(said.contains("PACKAGE LIST"), "{said}");
}

/// A project in a new directory, its root canonical, whose `Main` is
/// `main`.
fn project(main: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), main).unwrap();
    (work, root)
}

/// `nomi build src/Main.elm` with `args` in `root`, which must succeed.
fn build(cache: &Path, root: &Path, args: &[&str]) {
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let mut build = offline(program, cache);
    build
        .current_dir(root)
        .args(["build", "src/Main.elm"])
        .args(args);
    let built = build.output().unwrap();
    assert!(built.status.success(), "{built:?}");
}

/// What `node script` in `root` writes to standard output, once it
/// succeeds.
fn node(root: &Path, script: &str) -> String {
    let ran = Command::new("node")
        .arg(script)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(ran.status.success(), "{ran:?}");
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

/// A worker built into an `.mjs` file is an ES module exporting `Elm`:
/// Node imports it, and a message sent to its incoming port comes back
/// on its outgoing one before `send` returns, as it does built for Node
/// on the fiber runtime.
#[test]
fn a_module_output_is_imported_by_node_and_answers_on_its_ports() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "port module Main exposing (main)


port requests : (String -> msg) -> Sub msg


port responses : String -> Cmd msg


main : Program () () String
main =
    Platform.worker
        { init = \\_ -> ( (), Cmd.none )
        , update = \\request model -> ( model, responses (String.toUpper request) )
        , subscriptions = \\_ -> requests identity
        }
",
    );
    build(cache.path(), &root, &["--output=dist/main.mjs"]);
    // An IIFE, as a Node module of one `main` would start it.
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/node.js"],
    );
    // The outgoing port's manager sleeps a moment after each message, as
    // upstream's does, so the driver tries until it finds it ready.
    let run = "const responses = [];
app.ports.responses.subscribe((response) => responses.push(response));
const attempt = (left) => {
  responses.length = 0;
  app.ports.requests.send('ping');
  if (responses.length > 0) console.log(responses.join());
  else if (left > 0) setTimeout(() => attempt(left - 1), 20);
};
attempt(50);
";
    let drivers = [
        ("browser.mjs", "import { Elm } from './dist/main.mjs';"),
        ("node.cjs", "const { Elm } = require('./dist/node.js');"),
    ];
    for (driver, import) in drivers {
        let script = format!("{import}\nconst app = Elm.Main.init();\n{run}");
        fs::write(root.join(driver), script).unwrap();
        assert_eq!(node(&root, driver), "PING\n", "{driver}");
    }
    let node_js = fs::read_to_string(root.join("dist/node.js")).unwrap();
    assert!(node_js.contains("var _Runtime = "));
}

/// A worker built for Node into an `.mjs` file starts when run; built
/// for the browser, the module runs nothing.
#[test]
fn a_node_entry_starts_its_main_and_a_plain_module_runs_nothing() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "module Main exposing (main)


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( Debug.log \"started\" (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/entry.mjs"],
    );
    assert_eq!(node(&root, "dist/entry.mjs"), "started: ()\n");
    build(cache.path(), &root, &["--output=dist/plain.mjs"]);
    assert_eq!(node(&root, "dist/plain.mjs"), "");
}

/// Every file under `dir`, by its path from `dir`, with its contents.
fn files(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut dirs = vec![dir.to_owned()];
    while let Some(at) = dirs.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_owned();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// A project built for the browser, then for Node, finds everything it
/// built already: nothing in its artifacts or nomi's cache names a
/// platform.
#[test]
fn building_for_another_platform_writes_no_new_artifact() {
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "module Main exposing (main)

import Page


main : Program () String ()
main =
    Platform.worker
        { init = \\_ -> ( Page.title, Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/html": "1.0.1", "elm/json": "1.1.4"}, "indirect": {"elm/virtual-dom": "1.0.5"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("src/Page.elm"),
        "module Page exposing (title, view)\n\nimport Html exposing (Html)\n\n\ntitle : String\ntitle =\n    \"Home\"\n\n\nview : Html msg\nview =\n    Html.div [] [ Html.text title ]\n",
    )
    .unwrap();
    let artifacts = || {
        let mut project = files(&root);
        project.retain(|path, _| !path.starts_with("dist"));
        (files(cache.path()), project)
    };
    build(cache.path(), &root, &["--output=dist/browser.mjs"]);
    let browser = artifacts();
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/node.mjs"],
    );
    assert!(browser.0.keys().any(|path| path.starts_with("artifacts")));
    assert!(
        browser == artifacts(),
        "a build for Node changed the artifacts"
    );
}

/// A kernel Node replaces, in a release whose file nomi doesn't know, is
/// not available on Node: the build fails naming the release, where the
/// browser's still builds.
#[test]
fn an_unknown_release_of_a_replaced_kernel_fails_for_node() {
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "module Main exposing (main)


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    let scheduler = (files(cache.path()).into_keys())
        .find(|path| path.ends_with("src/Elm/Kernel/Scheduler.js"))
        .map(|path| cache.path().join(path))
        .unwrap();
    let mut text = fs::read_to_string(&scheduler).unwrap();
    text.push_str("\n// A release nomi doesn't know.\n");
    fs::write(&scheduler, text).unwrap();
    build(cache.path(), &root, &["--output=dist/browser.js"]);
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let built = offline(program, cache.path())
        .current_dir(&root)
        .args([
            "build",
            "src/Main.elm",
            "--platform=node",
            "--output=dist/node.js",
        ])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&built.stdout) + String::from_utf8_lossy(&built.stderr);
    assert!(!built.status.success(), "{said}");
    assert!(said.contains("NOT AVAILABLE ON NODE"), "{said}");
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        said.contains("uses Elm.Kernel.Scheduler of elm/core 1.0.5, a file"),
        "{said}"
    );
    let chain = "main → Platform.worker → Elm.Kernel.Platform → Elm.Kernel.Scheduler";
    assert!(said.contains(chain), "{said}");
    assert!(!root.join("dist/node.js").exists());
}

/// A worker that sleeps `sleep` ms, takes three ticks of `Time.every
/// every`, then reads `Time.now`, logging each step and whether the
/// program's clock had passed it. Host timers keep whole milliseconds,
/// and may fire up to one early by `Date.now`, so each of the four waits
/// is allowed one.
fn clocked_worker(sleep: u32, every: u32) -> String {
    format!(
        "module Main exposing (main)

import Process
import Task
import Time


type Msg
    = Start Time.Posix
    | Slept
    | Tick Time.Posix
    | Now Time.Posix


type alias Model =
    {{ start : Int, ticks : Int, ticking : Bool }}


main : Program () Model Msg
main =
    Platform.worker
        {{ init = \\_ -> ( Model 0 0 False, Task.perform Start Time.now )
        , update = update
        , subscriptions = \\model -> if model.ticking then Time.every {every} Tick else Sub.none
        }}


passed : Model -> Int -> Time.Posix -> Bool
passed model ms time =
    Time.posixToMillis time - model.start >= ms - 4


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        Start time ->
            ( {{ model | start = Time.posixToMillis time }}, Task.perform (\\_ -> Slept) (Process.sleep {sleep}) )

        Slept ->
            ( {{ model | ticking = Debug.log \"slept\" True }}, Cmd.none )

        Tick time ->
            let
                ticks =
                    model.ticks + 1

                _ =
                    Debug.log (\"tick \" ++ String.fromInt ticks) (passed model ({sleep} + ticks * {every}) time)
            in
            if ticks == 3 then
                ( {{ model | ticks = ticks, ticking = False }}, Task.perform Now Time.now )

            else
                ( {{ model | ticks = ticks }}, Cmd.none )

        Now time ->
            ( Debug.log \"now\" (passed model ({sleep} + 3 * {every}) time) |> always model, Cmd.none )
"
    )
}

/// Sleeping an hour and taking three `Time.every 1000` ticks takes no
/// real time on the virtual clock, and logs what the same program, with
/// waits of a few milliseconds, logs on the real clock.
#[test]
fn a_node_program_sleeps_and_ticks_on_its_clock() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let expected = "slept: True\ntick 1: True\ntick 2: True\ntick 3: True\nnow: True\n";
    for (sleep, every, clock) in [(3_600_000, 1000, "virtual"), (50, 5, "real")] {
        let (_work, root) = project(&clocked_worker(sleep, every));
        fs::write(
            root.join("elm.json"),
            r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "elm/time": "1.0.0"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
        )
        .unwrap();
        build(
            cache.path(),
            &root,
            &["--platform=node", "--output=dist/main.mjs"],
        );
        let started = std::time::Instant::now();
        let mut node = Command::new("node")
            .arg("dist/main.mjs")
            .env("NOMI_CLOCK", clock)
            .current_dir(&root)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        while node.try_wait().unwrap().is_none() && started.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = node.kill();
        let ran = node.wait_with_output().unwrap();
        let took = started.elapsed();
        assert!(ran.status.success(), "{clock}: {ran:?} after {took:?}");
        assert_eq!(String::from_utf8_lossy(&ran.stdout), expected, "{clock}");
        if clock == "virtual" {
            assert!(took.as_millis() < 1000, "the virtual clock took {took:?}");
        }
    }
}
