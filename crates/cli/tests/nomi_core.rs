//! nomi/core's `Nomi.program`, bundled with nomi: built offline from the
//! bundle, it is a Bun entry unless the browser is asked for, and runs
//! its task as the process, which exits with how the task ended, or at
//! once, naming it, when its host isn't Bun 1.4 or later; joining a fiber that dies is the joiner's defect, and its fibers share refs
//! and queues (`nomi_test.rs` runs its concurrent tasks, which take hours,
//! on nomi/test's virtual clock); `Nomi.log` writes to stderr. nomi/server,
//! bundled beside it, serves HTTP to a loopback client on the port it says
//! it took, a request per fiber, and drains on SIGTERM.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nomi_testkit::http::{self, Response};
use nomi_testkit::process::lines;
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

/// A server on the port its first argument names, or one it picks,
/// printing which once it listens, taking bodies up to the bytes its second
/// names, or `defaults`'s; answering `/json` with the field `n` of its
/// JSON body plus one, or the `BodyError`; `/header/<name>` with what it
/// reads of that header; `/cookies` with two cookies; and any other path
/// with what it reads of the request, in the body and the header `X-Echo`.
const SERVER: &str = r#"module Main exposing (main)

import Json.Decode as Decode
import Json.Encode as Encode
import Nomi exposing (Env)
import Nomi.Server as Server exposing (Request, Response)
import Nomi.Stdio as Stdio
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                defaults =
                    Server.defaults

                number i default =
                    List.drop i env.args |> List.head |> Maybe.andThen String.toInt |> Maybe.withDefault default
            in
            Server.listen { defaults | port_ = number 0 0, maxBodyBytes = number 1 defaults.maxBodyBytes, onListen = listening env } handle env.net
                |> Task.mapError Server.errorToString


listening : Env -> Int -> Task x ()
listening env port_ =
    Stdio.printLine ("listening on " ++ String.fromInt port_) env.stdout
        |> Task.onError (\_ -> Task.succeed ())


handle : Request -> Task Never Response
handle req =
    case Server.path req of
        [ "json" ] ->
            Server.jsonBody (Decode.field "n" Decode.int) req
                |> Task.map (\n -> Server.json 200 (Encode.object [ ( "n", Encode.int (n + 1) ) ]))
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        [ "header", name ] ->
            Task.succeed (Server.text 200 (Debug.toString (Server.header name req)))

        [ "cookies" ] ->
            Task.succeed (Server.empty 204 |> Server.addHeader "Set-Cookie" "a=1" |> Server.addHeader "X-Between" "1" |> Server.addHeader "Set-Cookie" "b=2")

        _ ->
            let
                echo =
                    Debug.toString ( Server.method req, Server.path req, ( Server.query "q" req, Server.header "X-Test" req ) )
            in
            Task.succeed (Server.text 200 echo |> Server.withHeader "X-Echo" echo)
"#;

/// A server on the port its first argument names, or one it picks,
/// printing which once it listens, draining for the milliseconds its second
/// names. `/hang` says so and never answers, with
/// a finalizer that says so too; `/slow` says so and answers with its
/// body; `/todo` dies; any other path is `ok`.
const LIFECYCLE: &str = r#"module Main exposing (main)

import Nomi exposing (Env)
import Nomi.Server as Server exposing (Request, Response)
import Nomi.Stdio as Stdio
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                defaults =
                    Server.defaults

                number i =
                    List.drop i env.args |> List.head |> Maybe.andThen String.toInt |> Maybe.withDefault 0
            in
            Server.listen { defaults | port_ = number 0, drainMilliseconds = number 1, onListen = listening env } handle env.net
                |> Task.mapError Server.errorToString


listening : Env -> Int -> Task x ()
listening env port_ =
    Stdio.printLine ("listening on " ++ String.fromInt port_) env.stdout
        |> Task.onError (\_ -> Task.succeed ())


handle : Request -> Task Never Response
handle req =
    case Server.path req of
        [ "hang" ] ->
            say "hanging"
                |> Task.andThen (\_ -> Process.sleep 3600000)
                |> Task.map (\_ -> Server.text 200 "woke")
                |> Nomi.Task.ensure (say "hang finalized")

        [ "slow" ] ->
            say "slow"
                |> Task.andThen (\_ -> Server.body req)
                |> Task.map (Server.text 200)
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        [ "todo" ] ->
            Debug.todo "boom"

        _ ->
            Task.succeed (Server.text 200 "ok")


say : String -> Task x ()
say line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())
"#;

/// An application of `program` on nomi/core and nomi/server, in a new
/// directory.
fn project(program: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/server": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8", "elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
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

/// A server serving on the port it picked, stopped when dropped.
struct Serving {
    port: u16,
    server: std::process::Child,
    /// The lines the server prints on standard output.
    said: mpsc::Receiver<String>,
    _work: tempfile::TempDir,
}

impl Serving {
    fn start() -> Serving {
        Serving::of(SERVER, &[])
    }

    /// `program` built and serving on port 0, with `args` after it, once
    /// it says which port it took.
    fn of(program: &str, args: &[&str]) -> Serving {
        let (work, root) = built(program);
        let mut server = bun()
            .args(["dist/main.mjs", "0"])
            .args(args)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let said = lines(server.stdout.take().unwrap());
        let mut serving = Serving {
            port: 0,
            server,
            said,
            _work: work,
        };
        let line = (serving.said)
            .recv_timeout(Duration::from_secs(10))
            .expect("the server says where it listens");
        let port = line.strip_prefix("listening on ").expect(&line);
        serving.port = port.parse().unwrap();
        assert_ne!(serving.port, 0);
        serving
    }

    /// Waits for the server to print `line`.
    fn says(&self, line: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match (self.said).recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(said) if said == line => return,
                Ok(_) => {}
                Err(e) => panic!("the server didn't print {line:?}: {e}"),
            }
        }
    }

    /// A new connection, once the server takes one.
    fn connect(&self) -> TcpStream {
        let started = Instant::now();
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", self.port)) {
                Ok(stream) => break stream,
                Err(e) if started.elapsed() > Duration::from_secs(10) => panic!("{e}"),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    /// `request` sent on a new connection, and the answer.
    fn send(&self, request: &[u8]) -> Response {
        let mut stream = self.connect();
        stream.write_all(request).unwrap();
        http::read(stream).unwrap()
    }

    /// Sends the server SIGTERM.
    fn terminate(&self) {
        let sent = Command::new("kill")
            .args(["-TERM".to_owned(), self.server.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
    }

    /// The server's exit code once it exits, within 15 seconds, and the
    /// lines of standard error it wrote.
    fn exit(&mut self) -> (Option<i32>, Vec<String>) {
        let started = Instant::now();
        while self.server.try_wait().unwrap().is_none() && started.elapsed().as_secs() < 15 {
            std::thread::sleep(Duration::from_millis(10));
        }
        let code = self
            .server
            .try_wait()
            .unwrap()
            .and_then(|status| status.code());
        let mut stderr = String::new();
        if code.is_some() {
            let mut pipe = self.server.stderr.take().unwrap();
            pipe.read_to_string(&mut stderr).unwrap();
        }
        let lines = (stderr.lines())
            .filter(|line| !line.starts_with("Compiled in DEV mode."))
            .map(str::to_owned)
            .collect();
        (code, lines)
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

#[test]
fn a_server_reads_each_method_path_query_and_header_and_stops_on_sigterm() {
    nomi_testkit::require_bun();
    let mut serving = Serving::start();
    let methods = [
        ("GET", "Get"),
        ("HEAD", "Head"),
        ("POST", "Post"),
        ("PUT", "Put"),
        ("PATCH", "Patch"),
        ("DELETE", "Delete"),
        ("OPTIONS", "Options"),
        ("PURGE", "OtherMethod \"PURGE\""),
    ];
    for (method, read) in methods {
        let sent = http::closing(
            method,
            "/echo/a%20b//c/?q=x%26y&q=2&r",
            "x-TeSt: Hi\r\n",
            b"",
        );
        let sent = serving.send(&sent);
        let (status, body) = (sent.status, sent.text());
        let echo = format!("({read},[\"echo\",\"a b\",\"c\"],(Just \"x&y\",Just \"Hi\"))");
        assert_eq!(status, 200, "{method}");
        assert_eq!(sent.header("x-echo"), Some(echo.as_str()), "{method}");
        let expected = if method == "HEAD" { "" } else { &echo };
        assert_eq!(body, expected, "{method}");
        assert_eq!(
            sent.header("content-type"),
            Some("text/plain; charset=utf-8")
        );
    }
    let sent = serving.send(&http::closing("GET", "/", "", b""));
    let body = sent.text();
    assert_eq!(body, "(Get,[],(Nothing,Nothing))");
    serving.terminate();
    assert_eq!(serving.exit(), (Some(143), vec![]));
}

#[test]
fn a_server_decodes_json_bodies_and_refuses_large_and_bad_ones() {
    nomi_testkit::require_bun();
    let serving = Serving::start();
    let sent = serving.send(&http::closing("POST", "/json", "", br#"{"n": 41}"#));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, r#"{"n":42}"#));
    assert_eq!(sent.header("content-type"), Some("application/json"));
    let sent = serving.send(&http::closing("POST", "/json", "", br#"{"n":"#));
    let (status, body) = (sent.status, sent.text());
    assert_eq!(status, 400);
    assert!(
        body.starts_with("BadJson \"Problem with the given value"),
        "{body}"
    );
    let sent = serving.send(&http::closing("POST", "/json", "", br#"{"n": "x"}"#));
    let body = sent.text();
    assert!(
        body.starts_with("BadJson \"Problem with the value at json.n"),
        "{body}"
    );
    // A body past 1 MiB, whether its length is declared or not.
    let large = vec![b' '; 1024 * 1024 + 1];
    let sent = serving.send(&http::closing("POST", "/json", "", &large));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (400, "TooLarge"));
    let mut chunked = b"POST /json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..2 {
        chunked.extend_from_slice(format!("{:x}\r\n", large.len()).as_bytes());
        chunked.extend_from_slice(&large);
        chunked.extend_from_slice(b"\r\n");
    }
    chunked.extend_from_slice(b"0\r\n\r\n");
    let sent = serving.send(&chunked);
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (400, "TooLarge"));
    // At the limit, it is read.
    let mut whole = vec![b' '; 1024 * 1024 - 9];
    whole.extend_from_slice(br#"{"n": 1}"#);
    whole.push(b' ');
    let sent = serving.send(&http::closing("POST", "/json", "", &whole));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, r#"{"n":2}"#));
}

#[test]
fn a_server_on_a_taken_port_fails_with_address_in_use() {
    nomi_testkit::require_bun();
    let (_work, root) = built(SERVER);
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let ran = bun()
        .args(["dist/main.mjs", &port.to_string()])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(
        errors(&ran),
        [format!(
            "nomi: the program failed: 127.0.0.1:{port}: address already in use"
        )]
    );
}

#[test]
fn a_server_on_a_port_out_of_range_fails_saying_so() {
    nomi_testkit::require_bun();
    let (_work, root) = built(SERVER);
    let ran = bun()
        .args(["dist/main.mjs", "70000"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(
        errors(&ran),
        ["nomi: the program failed: port 70000 is out of range: a port is 0 to 65535"]
    );
}

#[test]
fn a_server_takes_bodies_up_to_its_max_body_bytes() {
    nomi_testkit::require_bun();
    let serving = Serving::of(SERVER, &["16"]);
    let sent = serving.send(&http::closing("POST", "/json", "", br#"{"n": 1}        "#));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, r#"{"n":2}"#));
    let sent = serving.send(&http::closing("POST", "/json", "", br#"{"n": 1}         "#));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (400, "TooLarge"));
}

#[test]
fn a_server_reads_repeated_headers_by_fetchs_rules_and_sends_each_added_one() {
    nomi_testkit::require_bun();
    let serving = Serving::start();
    for (name, read) in [
        ("cookie", r#"Just "a=1; b=2""#),
        ("content-type", r#"Just "a, b""#),
        ("x-many", r#"Just "1, 2""#),
    ] {
        let sent = http::closing(
            "GET",
            &format!("/header/{name}"),
            "Cookie: a=1\r\nCookie: b=2\r\nContent-Type: a\r\nContent-Type: b\r\nX-Many: 1\r\nX-Many: 2\r\n",
            b"",
        );
        let sent = serving.send(&sent);
        let body = sent.text();
        assert_eq!(body, read, "{name}");
    }
    let sent = serving.send(&http::closing("GET", "/cookies", "", b""));
    let status = sent.status;
    assert_eq!(status, 204);
    // Each `Set-Cookie` apart, as `respond` gives them.
    let added: Vec<_> = (sent.headers.iter())
        .filter(|(name, _)| name == "set-cookie" || name == "x-between")
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        added,
        [
            ("set-cookie", "a=1"),
            ("set-cookie", "b=2"),
            ("x-between", "1")
        ]
    );
}

#[test]
fn a_client_that_disconnects_interrupts_its_request_and_runs_its_finalizers() {
    nomi_testkit::require_bun();
    let mut serving = Serving::of(LIFECYCLE, &["5000"]);
    let mut stream = serving.connect();
    stream
        .write_all(&http::closing("GET", "/hang", "", b""))
        .unwrap();
    serving.says("hanging: ()");
    drop(stream);
    serving.says("hang finalized: ()");
    let sent = serving.send(&http::closing("GET", "/", "", b""));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, "ok"));
    serving.terminate();
    assert_eq!(serving.exit(), (Some(143), vec![]));
}

#[test]
fn a_handler_that_dies_answers_500_writes_one_line_and_the_server_serves_on() {
    nomi_testkit::require_bun();
    let mut serving = Serving::of(LIFECYCLE, &["5000"]);
    let sent = serving.send(&http::closing("GET", "/todo?x=1", "X-Gone: yes\r\n", b""));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (500, "internal server error"));
    assert_eq!(
        sent.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    let sent = serving.send(&http::closing("GET", "/", "", b""));
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, "ok"));
    serving.terminate();
    let line = LIFECYCLE
        .lines()
        .position(|line| line.contains("Debug.todo"))
        .unwrap()
        + 1;
    assert_eq!(
        serving.exit(),
        (
            Some(143),
            vec![format!(
                "nomi: request GET /todo?x=1 died: Error: TODO in module `Main` on line {line} boom"
            )]
        )
    );
}

#[test]
fn sigterm_refuses_new_connections_lets_a_response_in_flight_complete_and_exits_143() {
    nomi_testkit::require_bun();
    let mut serving = Serving::of(LIFECYCLE, &["30000"]);
    // The handler waits for the body, which is sent once the server
    // refuses connections.
    let mut stream = serving.connect();
    let head =
        b"POST /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 4\r\n\r\n";
    stream.write_all(head).unwrap();
    serving.says("slow: ()");
    serving.terminate();
    let signalled = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", serving.port)) {
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => break,
            _ if signalled.elapsed() > Duration::from_secs(10) => panic!("still accepting"),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    stream.write_all(b"done").unwrap();
    let sent = http::read(stream).unwrap();
    let (status, body) = (sent.status, sent.text());
    assert_eq!((status, body.as_str()), (200, "done"));
    assert_eq!(serving.exit(), (Some(143), vec![]));
    // Well before the drain's 30 seconds pass.
    assert!(signalled.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_response_written_during_the_drain_closes_its_connection() {
    nomi_testkit::require_bun();
    let mut serving = Serving::of(LIFECYCLE, &["30000"]);
    let mut stream = serving.connect();
    let head = b"POST /slow HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\n\r\n";
    stream.write_all(head).unwrap();
    serving.says("slow: ()");
    serving.terminate();
    let signalled = Instant::now();
    while TcpStream::connect(("127.0.0.1", serving.port)).is_ok() {
        assert!(
            signalled.elapsed() < Duration::from_secs(10),
            "still accepting"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The body, and a request after it on the same connection, which the
    // client mustn't be told it can send.
    stream
        .write_all(b"doneGET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        head.to_ascii_lowercase().contains("\r\nconnection: close"),
        "{answer}"
    );
    assert_eq!(body, "done");
    assert_eq!(serving.exit(), (Some(143), vec![]));
    assert!(signalled.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_request_still_running_when_the_drain_ends_is_interrupted_and_its_connection_closed() {
    nomi_testkit::require_bun();
    let mut serving = Serving::of(LIFECYCLE, &["1000"]);
    let mut stream = serving.connect();
    stream
        .write_all(&http::closing("GET", "/hang", "", b""))
        .unwrap();
    serving.says("hanging: ()");
    serving.terminate();
    let signalled = Instant::now();
    let mut answer = Vec::new();
    // Closed, or reset, without a response.
    let _ = stream.read_to_end(&mut answer);
    assert_eq!(String::from_utf8_lossy(&answer), "");
    serving.says("hang finalized: ()");
    // Exited by itself, so with no line about the lifecycle's deadline.
    assert_eq!(serving.exit(), (Some(143), vec![]));
    assert!(signalled.elapsed() >= Duration::from_millis(900));
}
