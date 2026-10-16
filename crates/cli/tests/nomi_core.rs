//! nomi/core's `Nomi.program`, bundled with nomi: built offline from the
//! bundle, it is a Node entry unless the browser is asked for, and runs
//! its task as the process, which exits with how the task ended; joining
//! a fiber that dies is the joiner's defect, and its fibers share refs
//! and queues (`nomi_test.rs` runs its concurrent tasks, which take hours,
//! on nomi/test's virtual clock). nomi/server, bundled beside it, serves
//! HTTP to a loopback client, a request per fiber, and drains on SIGTERM.
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

/// A server on the port its argument names, answering `/json` with the
/// field `n` of its JSON body plus one, or the `BodyError`, and any other
/// path with what it reads of the request, in the body and the header
/// `X-Echo`.
const SERVER: &str = r#"module Main exposing (main)

import Json.Decode as Decode
import Json.Encode as Encode
import Nomi
import Nomi.Server as Server exposing (Request, Response)
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                defaults =
                    Server.defaults

                port_ =
                    List.head env.args |> Maybe.andThen String.toInt |> Maybe.withDefault 0
            in
            Server.listen { defaults | port_ = port_ } handle env.net
                |> Task.mapError Server.errorToString


handle : Request -> Task Never Response
handle req =
    case Server.path req of
        [ "json" ] ->
            Server.jsonBody (Decode.field "n" Decode.int) req
                |> Task.map (\n -> Server.json 200 (Encode.object [ ( "n", Encode.int (n + 1) ) ]))
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        _ ->
            let
                echo =
                    Debug.toString ( Server.method req, Server.path req, ( Server.query "q" req, Server.header "X-Test" req ) )
            in
            Task.succeed (Server.text 200 echo |> Server.withHeader "X-Echo" echo)
"#;

/// A server on the port its first argument names, draining for the
/// milliseconds its second names. `/hang` says so and never answers, with
/// a finalizer that says so too; `/slow` says so and answers with its
/// body; `/todo` dies; any other path is `ok`.
const LIFECYCLE: &str = r#"module Main exposing (main)

import Nomi
import Nomi.Server as Server exposing (Request, Response)
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
            Server.listen { defaults | port_ = number 0, drainMilliseconds = number 1 } handle env.net
                |> Task.mapError Server.errorToString


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
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0", "nomi/server": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
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
fn joining_a_fiber_that_dies_is_the_joiners_defect() {
    nomi_testkit::require_node();
    let (_work, root) = built(DEFECT);
    let ran = node()
        .arg("dist/main.mjs")
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

#[test]
fn fibers_share_refs_and_queues_in_order_and_interruption_loses_no_item() {
    nomi_testkit::require_node();
    let (_work, root) = built(SHARED);
    let ran = node()
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

/// A port on the loopback interface that nothing listens on.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// A server serving on a port of its own, stopped when dropped.
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

    /// `program` built and serving, with `args` after its port.
    fn of(program: &str, args: &[&str]) -> Serving {
        let (work, root) = built(program);
        let port = free_port();
        let mut server = node()
            .arg("dist/main.mjs")
            .arg(port.to_string())
            .args(args)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(server.stdout.take().unwrap());
        let (lines, said) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        Serving {
            port,
            server,
            said,
            _work: work,
        }
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

    /// `request` sent on a new connection; the response's status, headers
    /// with lower-case names, and body.
    fn send(&self, request: &[u8]) -> (u16, Vec<(String, String)>, String) {
        let mut stream = self.connect();
        stream.write_all(request).unwrap();
        response(&mut stream)
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

/// The response read from `stream` to its end: its status, headers with
/// lower-case names, and body.
fn response(stream: &mut TcpStream) -> (u16, Vec<(String, String)>, String) {
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap();
    let headers = lines
        .map(|line| line.split_once(": ").unwrap())
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    (status.parse().unwrap(), headers, body.to_owned())
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

/// A request closing its connection, with `headers`, each ending in a
/// line break, and `body`.
fn request(method: &str, target: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{headers}Content-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

/// The value of the header `name` among `headers`.
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    (headers.iter())
        .find(|(n, _)| n == name)
        .map(|(_, value)| value.as_str())
}

#[test]
fn a_server_reads_each_method_path_query_and_header_and_stops_on_sigterm() {
    nomi_testkit::require_node();
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
        let sent = request(
            method,
            "/echo/a%20b//c/?q=x%26y&q=2&r",
            "x-TeSt: Hi\r\n",
            b"",
        );
        let (status, headers, body) = serving.send(&sent);
        let echo = format!("({read},[\"echo\",\"a b\",\"c\"],(Just \"x&y\",Just \"Hi\"))");
        assert_eq!(status, 200, "{method}");
        assert_eq!(header(&headers, "x-echo"), Some(echo.as_str()), "{method}");
        let expected = if method == "HEAD" { "" } else { &echo };
        assert_eq!(body, expected, "{method}");
        assert_eq!(
            header(&headers, "content-type"),
            Some("text/plain; charset=utf-8")
        );
    }
    let (_, _, body) = serving.send(&request("GET", "/", "", b""));
    assert_eq!(body, "(Get,[],(Nothing,Nothing))");
    serving.terminate();
    assert_eq!(serving.exit(), (Some(143), vec![]));
}

#[test]
fn a_server_decodes_json_bodies_and_refuses_large_and_bad_ones() {
    nomi_testkit::require_node();
    let serving = Serving::start();
    let (status, headers, body) = serving.send(&request("POST", "/json", "", br#"{"n": 41}"#));
    assert_eq!((status, body.as_str()), (200, r#"{"n":42}"#));
    assert_eq!(header(&headers, "content-type"), Some("application/json"));
    let (status, _, body) = serving.send(&request("POST", "/json", "", br#"{"n":"#));
    assert_eq!(status, 400);
    assert!(
        body.starts_with("BadJson \"Problem with the given value"),
        "{body}"
    );
    let (_, _, body) = serving.send(&request("POST", "/json", "", br#"{"n": "x"}"#));
    assert!(
        body.starts_with("BadJson \"Problem with the value at json.n"),
        "{body}"
    );
    // A body past 1 MiB, whether its length is declared or not.
    let large = vec![b' '; 1024 * 1024 + 1];
    let (status, _, body) = serving.send(&request("POST", "/json", "", &large));
    assert_eq!((status, body.as_str()), (400, "TooLarge"));
    let mut chunked = b"POST /json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..2 {
        chunked.extend_from_slice(format!("{:x}\r\n", large.len()).as_bytes());
        chunked.extend_from_slice(&large);
        chunked.extend_from_slice(b"\r\n");
    }
    chunked.extend_from_slice(b"0\r\n\r\n");
    let (status, _, body) = serving.send(&chunked);
    assert_eq!((status, body.as_str()), (400, "TooLarge"));
    // At the limit, it is read.
    let mut whole = vec![b' '; 1024 * 1024 - 9];
    whole.extend_from_slice(br#"{"n": 1}"#);
    whole.push(b' ');
    let (status, _, body) = serving.send(&request("POST", "/json", "", &whole));
    assert_eq!((status, body.as_str()), (200, r#"{"n":2}"#));
}

#[test]
fn a_server_on_a_taken_port_fails_with_address_in_use() {
    nomi_testkit::require_node();
    let (_work, root) = built(SERVER);
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let ran = node()
        .args(["dist/main.mjs", &port.to_string()])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(
        errors(&ran),
        ["nomi: the program failed: the address is already in use"]
    );
}

#[test]
fn a_client_that_disconnects_interrupts_its_request_and_runs_its_finalizers() {
    nomi_testkit::require_node();
    let mut serving = Serving::of(LIFECYCLE, &["5000"]);
    let mut stream = serving.connect();
    stream.write_all(&request("GET", "/hang", "", b"")).unwrap();
    serving.says("hanging: ()");
    drop(stream);
    serving.says("hang finalized: ()");
    let (status, _, body) = serving.send(&request("GET", "/", "", b""));
    assert_eq!((status, body.as_str()), (200, "ok"));
    serving.terminate();
    assert_eq!(serving.exit(), (Some(143), vec![]));
}

#[test]
fn a_handler_that_dies_answers_500_writes_one_line_and_the_server_serves_on() {
    nomi_testkit::require_node();
    let mut serving = Serving::of(LIFECYCLE, &["5000"]);
    let (status, headers, body) =
        serving.send(&request("GET", "/todo?x=1", "X-Gone: yes\r\n", b""));
    assert_eq!((status, body.as_str()), (500, "internal server error"));
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/plain; charset=utf-8")
    );
    let (status, _, body) = serving.send(&request("GET", "/", "", b""));
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
    nomi_testkit::require_node();
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
    let (status, _, body) = response(&mut stream);
    assert_eq!((status, body.as_str()), (200, "done"));
    assert_eq!(serving.exit(), (Some(143), vec![]));
    // Well before the drain's 30 seconds pass.
    assert!(signalled.elapsed() < Duration::from_secs(20));
}

#[test]
fn a_request_still_running_when_the_drain_ends_is_interrupted_and_its_connection_closed() {
    nomi_testkit::require_node();
    let mut serving = Serving::of(LIFECYCLE, &["1000"]);
    let mut stream = serving.connect();
    stream.write_all(&request("GET", "/hang", "", b"")).unwrap();
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
