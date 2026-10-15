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

/// A worker built for Node into an `.mjs` file starts when run, and with
/// nothing live exits 0 on its own; built for the browser, the module
/// runs nothing.
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
/// browser's still builds. elm/http 1.0.0's `Http.js` is such a file.
#[test]
fn an_unknown_release_of_a_replaced_kernel_fails_for_node() {
    let worker = "module Main exposing (main)


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
";
    unknown_release_fails(
        worker,
        None,
        "Scheduler",
        "elm/core 1.0.5",
        "main → Platform.worker → Elm.Kernel.Platform → Elm.Kernel.Scheduler",
    );
    unknown_release_fails(
        HTTP_WORKER,
        Some(HTTP_APP),
        "Http",
        "elm/http 2.0.0",
        "main → Main.requests → Http.expectStringResponse → Elm.Kernel.Http",
    );
}

/// Builds `main`, with `elm_json` if given, for the browser and then for
/// Node with the kernel `module` changed, which must fail naming
/// `release` and the `chain` to it.
fn unknown_release_fails(
    main: &str,
    elm_json: Option<&str>,
    module: &str,
    release: &str,
    chain: &str,
) {
    let cache = website::fixture_cache();
    let (_work, root) = project(main);
    if let Some(elm_json) = elm_json {
        fs::write(root.join("elm.json"), elm_json).unwrap();
    }
    let file = format!("src/Elm/Kernel/{module}.js");
    let kernel = (files(cache.path()).into_keys())
        .find(|path| path.ends_with(&file))
        .map(|path| cache.path().join(path))
        .unwrap();
    let mut text = fs::read_to_string(&kernel).unwrap();
    text.push_str("\n// A release nomi doesn't know.\n");
    fs::write(&kernel, text).unwrap();
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
    let uses = format!("uses Elm.Kernel.{module} of {release}, a file");
    assert!(said.contains(&uses), "{said}");
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

/// A worker logging each tick of a `Time.every 20`.
const TICKING_WORKER: &str = "module Main exposing (main)

import Time


main : Program () Int Time.Posix
main =
    Platform.worker
        { init = \\_ -> ( 0, Cmd.none )
        , update = \\_ ticks -> ( Debug.log \"tick\" (ticks + 1), Cmd.none )
        , subscriptions = \\_ -> Time.every 20 identity
        }
";

/// A `Platform.worker` entry lives while its `Time.every` is live, and
/// SIGINT or SIGTERM shuts its runtime down and exits 130 or 143 within
/// the lifecycle's deadline of ten seconds. One with nothing live exits
/// 0 on its own (`a_node_entry_starts_its_main_and_a_plain_module_runs_nothing`).
#[test]
fn a_ticking_node_worker_runs_until_a_signal_and_exits_with_its_code() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(TICKING_WORKER);
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
    // Node itself: a version manager's `node` may exit 128 plus the
    // signal for a child the signal killed.
    let node = Command::new("node")
        .args(["-p", "process.execPath"])
        .output()
        .unwrap();
    let node = String::from_utf8(node.stdout).unwrap();
    for (signal, code) in [("TERM", 143), ("INT", 130)] {
        let mut node = Command::new(node.trim())
            .arg("dist/main.mjs")
            .current_dir(&root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let (lines, ticks) = std::sync::mpsc::channel();
        let stdout = BufReader::new(node.stdout.take().unwrap());
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let _ = lines.send(line.unwrap());
            }
        });
        // Five ticks, a hundred milliseconds of nothing but a timer, and
        // it hasn't exited.
        for tick in 1..=5 {
            let line = ticks.recv_timeout(Duration::from_secs(10));
            assert_eq!(line.as_deref(), Ok(&*format!("tick: {tick}")), "{signal}");
        }
        assert!(node.try_wait().unwrap().is_none(), "{signal}");
        let sent = Command::new("kill")
            .args([format!("-{signal}"), node.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
        let signalled = Instant::now();
        while node.try_wait().unwrap().is_none() && signalled.elapsed().as_secs() < 15 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = node.kill();
        let ran = node.wait_with_output().unwrap();
        let took = signalled.elapsed();
        assert_eq!(ran.status.code(), Some(code), "SIG{signal}: {ran:?}");
        assert!(took < Duration::from_secs(10), "SIG{signal} took {took:?}");
    }
}

/// The `elm.json` of an application on elm/http 2.0.0.
const HTTP_APP: &str = r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/http": "2.0.0", "elm/json": "1.1.4"}, "indirect": {"elm/file": "1.0.5", "elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#;

/// A worker that makes each request in turn against the server at its
/// flags, writing each `Result` to `log`, then cancels a request that is
/// receiving its body and asks the server whether its connection closed.
const HTTP_WORKER: &str = r#"port module Main exposing (main)

import Bytes.Decode
import Dict
import Http
import Json.Decode as D
import Json.Encode as E


port log : String -> Cmd msg


port done : () -> Cmd msg


type Msg
    = Got String
    | Progress Http.Progress


type alias Model =
    { base : String, left : List (Cmd Msg), tracking : Bool }


main : Program String Model Msg
main =
    Platform.worker
        { init = \base -> update (Got "start") (Model base (requests base) True)
        , update = update
        , subscriptions = \model -> if model.tracking then Http.track "hang" Progress else Sub.none
        }


got : String -> Result x a -> Msg
got label result =
    Got (label ++ ": " ++ Debug.toString result)


get : String -> String -> Maybe Float -> Maybe String -> Http.Expect Msg -> Cmd Msg
get base path timeout tracker expect =
    Http.request
        { method = "GET"
        , headers = [ Http.header "X-Ask" "elm" ]
        , url = base ++ path
        , body = Http.emptyBody
        , expect = expect
        , timeout = timeout
        , tracker = tracker
        }


metadata : Http.Response String -> Result String { status : Int, text : String, url : Bool, headers : List ( String, String ), body : String }
metadata response =
    case response of
        Http.GoodStatus_ m body ->
            Ok
                { status = m.statusCode
                , text = m.statusText
                , url = String.endsWith "/text" m.url
                , headers = Dict.toList (Dict.filter (\k _ -> String.startsWith "x-" k || k == "set-cookie") m.headers)
                , body = body
                }

        _ ->
            Err "not good"


requests : String -> List (Cmd Msg)
requests base =
    [ Http.get { url = base ++ "/text", expect = Http.expectString (got "get") }
    , get base "/text" Nothing Nothing (Http.expectStringResponse (got "metadata") metadata)
    , Http.post
        { url = base ++ "/echo"
        , body = Http.jsonBody (E.object [ ( "n", E.int 1 ) ])
        , expect = Http.expectJson (got "post") (D.map3 (\m t b -> ( m, t, b )) (D.field "method" D.string) (D.field "type" D.string) (D.field "body" D.string))
        }
    , Http.get { url = base ++ "/text", expect = Http.expectBytes (got "bytes") (Bytes.Decode.string 5) }
    , Http.get { url = base ++ "/text", expect = Http.expectWhatever (got "whatever") }
    , Http.get { url = base ++ "/missing", expect = Http.expectString (got "bad status") }
    , get base "/slow" (Just 50) Nothing (Http.expectString (got "timeout"))
    , Http.get { url = base ++ "/reset", expect = Http.expectString (got "network error") }
    , Http.get { url = "not a url", expect = Http.expectString (got "bad url") }
    , get base "/hang" Nothing (Just "hang") (Http.expectString (got "cancelled"))
    ]


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        Got line ->
            case model.left of
                next :: rest ->
                    ( { model | left = rest }, Cmd.batch [ log line, next ] )

                [] ->
                    ( model, Cmd.batch [ log line, done () ] )

        Progress progress ->
            ( { model | tracking = False }
            , Cmd.batch
                [ log ("progress: " ++ Debug.toString progress)
                , Http.cancel "hang"
                , get model.base "/closed" (Just 5000) Nothing (Http.expectString (got "closed"))
                ]
            )
"#;

/// The server `HTTP_WORKER` talks to, on a loopback port it passes as the
/// worker's flags; the program's standard output is what the worker logs.
const HTTP_DRIVER: &str = r#"const http = require('node:http');
const { Elm } = require('./dist/main.js');
let hung;
const server = http.createServer((req, res) => {
  switch (req.url) {
    case '/text':
      res.setHeader('X-Asked', req.headers['x-ask'] ?? '');
      res.setHeader('X-Two', ['a', 'b']);
      res.setHeader('Set-Cookie', 'k=v');
      return res.end('hello');
    case '/echo': {
      let body = '';
      req.on('data', (chunk) => body += chunk);
      return req.on('end', () => res.end(JSON.stringify({ method: req.method, type: req.headers['content-type'], body })));
    }
    case '/missing':
      res.statusCode = 404;
      return res.end('no');
    case '/slow':
      return;
    case '/reset':
      return req.socket.destroy();
    case '/hang':
      res.writeHead(200, { 'Content-Length': 10 });
      res.write('hello');
      hung = new Promise((resolve) => res.on('close', resolve));
      return;
    case '/closed':
      return hung.then(() => res.end('closed'));
  }
});
server.listen(0, '127.0.0.1', () => {
  const app = Elm.Main.init({ flags: `http://127.0.0.1:${server.address().port}` });
  app.ports.log.subscribe((line) => console.log(line));
  app.ports.done.subscribe(() => {
    server.closeAllConnections();
    server.close();
  });
});
"#;

/// elm/http on Node gives the `Result` the browser's kernel gives: for a
/// GET and a POST, each `expect`, the response's metadata, a bad status,
/// a timeout, a dropped connection and a bad URL. A cancelled request
/// closes its connection and answers nothing.
#[test]
fn a_node_program_makes_http_requests() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(HTTP_WORKER);
    fs::write(root.join("elm.json"), HTTP_APP).unwrap();
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/main.js"],
    );
    fs::write(root.join("driver.cjs"), HTTP_DRIVER).unwrap();
    let expected = r#"start
get: Ok "hello"
metadata: Ok { body = "hello", headers = [("x-asked","elm"),("x-two","a, b")], status = 200, text = "OK", url = True }
post: Ok ("POST","application/json","{\"n\":1}")
bytes: Ok "hello"
whatever: Ok ()
bad status: Err (BadStatus 404)
timeout: Err Timeout
network error: Err NetworkError
bad url: Err (BadUrl "not a url")
progress: Receiving { received = 5, size = Just 10 }
closed: Ok "closed"
"#;
    assert_eq!(node(&root, "driver.cjs"), expected);
}
