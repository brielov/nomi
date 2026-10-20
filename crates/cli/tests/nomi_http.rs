//! nomi/http's server and client, bundled with nomi, on Bun: a server
//! serves HTTP to a client on the port it says it took, a request per
//! fiber, streams bodies both ways, serves files and server-sent events
//! and TLS, and drains on SIGTERM; the client gets the same responses from
//! a socket and a loopback, within what its network allows.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nomi_testkit::http::{self, Response};
use nomi_testkit::process::lines;
use website::offline;

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
import Nomi.Http.Server as Server exposing (Request, Response)
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
            Server.bodyJson (Decode.field "n" Decode.int) req
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
import Nomi.Http.Server as Server exposing (Request, Response)
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
                |> Task.andThen (\_ -> Server.bodyText req)
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

/// A program serving one handler on a socket and on a loopback, sending
/// each the same requests with nomi/http's client and printing what came
/// back, without the headers about the connection; then the network's
/// narrowing; then a request to `/slow` given up by its fiber's timeout
/// and by its own, which the handler hears of on both.
const CLIENT: &str = r#"module Main exposing (main)

import Json.Encode as Encode
import Nomi exposing (Env)
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Http.Client as Client
import Nomi.Http.Server as Server exposing (Request, Response)
import Nomi.Net as Net exposing (Net)
import Nomi.Queue as Queue exposing (Queue)
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            Queue.unbounded
                |> Task.andThen
                    (\heard ->
                        serve env.net heard
                            |> Task.andThen
                                (\real ->
                                    Net.loopback
                                        |> Task.andThen (\loop -> serve loop heard |> Task.map (\port_ -> ( real, loop, port_ )))
                                )
                            |> Task.andThen
                                (\( real, loop, loopPort ) ->
                                    report env "socket" (exchanges env.net real)
                                        |> Task.andThen (\_ -> report env "loopback" (exchanges loop loopPort))
                                        |> Task.andThen (\_ -> report env "narrowed" (narrowed env real))
                                        |> Task.andThen (\_ -> interrupted env env.net real heard)
                                        |> Task.andThen (\_ -> interrupted env loop loopPort heard)
                                )
                    )


{-| A server of `handle` on the network, once it listens on a port.
-}
serve : Net -> Queue String -> Task String Int
serve net heard =
    let
        defaults =
            Server.defaults
    in
    Queue.unbounded
        |> Task.andThen
            (\ports ->
                Fiber.fork (Server.listen { defaults | port_ = 0, onListen = \port_ -> Queue.offer port_ ports } (handle heard) net)
                    |> Task.andThen (\_ -> Queue.take ports)
            )


handle : Queue String -> Request -> Task Never Response
handle heard req =
    case Server.path req of
        [] ->
            Task.succeed (Server.text 200 "hello")

        [ "echo" ] ->
            Server.bodyText req
                |> Task.map (\text -> Server.text 200 text |> Server.withHeader "X-Method" (Debug.toString (Server.method req)))
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        [ "json" ] ->
            Task.succeed (Server.json 201 (Encode.object [ ( "n", Encode.int 1 ) ]))

        [ "cookies" ] ->
            Task.succeed (Server.empty 204 |> Server.addHeader "Set-Cookie" "a=1" |> Server.addHeader "Set-Cookie" "b=2")

        [ "redirect" ] ->
            Task.succeed (Server.empty 302 |> Server.withHeader "Location" (Maybe.withDefault "/redirect" (Server.query "to" req)))

        [ "see-other" ] ->
            Task.succeed (Server.empty 303 |> Server.withHeader "Location" "/echo")

        [ "stream" ] ->
            Stream.fromBytes (Nomi.Bytes.fromString "streamed body") |> Task.map (Server.stream 200)

        [ "slow" ] ->
            Queue.offer "started" heard
                |> Task.andThen (\_ -> Process.sleep 3600000)
                |> Task.map (\_ -> Server.empty 204)
                |> Nomi.Task.ensure (Queue.offer "finalized" heard)

        _ ->
            Task.succeed (Server.text 404 "not found")


{-| The same requests to a server on `net`, each what came back.
-}
exchanges : Net -> Int -> List (Task x String)
exchanges net port_ =
    let
        base =
            "http://127.0.0.1:" ++ String.fromInt port_

        shown name req =
            Client.sendText req net
                |> Task.map (\response -> name ++ ": " ++ String.fromInt response.status ++ " " ++ Debug.toString (List.filter kept response.headers) ++ " " ++ Debug.toString response.body)
                |> Task.onError (\error -> Task.succeed (name ++ ": " ++ Debug.toString error))

        kept ( name, _ ) =
            not (List.member name [ "date", "content-length", "transfer-encoding", "connection", "keep-alive" ])
    in
    [ shown "get" (Client.get base)
    , shown "post text" (Client.post (base ++ "/echo") |> Client.withText "posted")
    , shown "put json" (Client.put (base ++ "/echo") |> Client.withJson (Encode.list Encode.int [ 1, 2 ]))
    , shown "json" (Client.get (base ++ "/json"))
    , shown "cookies" (Client.get (base ++ "/cookies"))
    , shown "redirect" (Client.get (base ++ "/redirect?to=/json"))
    , shown "see other" (Client.post (base ++ "/see-other") |> Client.withText "dropped")
    , shown "loop" (Client.get (base ++ "/redirect?to=/redirect"))
    , shown "stream" (Client.get (base ++ "/stream"))
    , shown "missing" (Client.get (base ++ "/missing"))
    , shown "bad url" (Client.get "not a url")
    , shown "refused" (Client.get "http://127.0.0.1:1/")
    ]


{-| What a narrowed network refuses.
-}
narrowed : Env -> Int -> List (Task x String)
narrowed env port_ =
    let
        base =
            "http://127.0.0.1:" ++ String.fromInt port_

        outcome name task =
            task |> Task.map (\_ -> name ++ ": Ok") |> Task.onError (\error -> Task.succeed (name ++ ": " ++ Debug.toString error))
    in
    [ outcome "only example.com" (Client.sendText (Client.get base) (Net.onlyHosts [ "example.com" ] env.net))
    , outcome "only 127.0.0.1" (Client.sendText (Client.get base) (Net.onlyHosts [ "127.0.0.1" ] env.net))
    , outcome "redirected to localhost" (Client.sendText (Client.get (base ++ "/redirect?to=http://localhost:" ++ String.fromInt port_ ++ "/")) (Net.onlyHosts [ "127.0.0.1" ] env.net))
    , outcome "narrowed twice" (Client.sendText (Client.get base) (Net.onlyHosts [ "127.0.0.1" ] (Net.onlyHosts [ "example.com" ] env.net)))
    , outcome "no listen" (Server.listen Server.defaults (\_ -> Task.succeed (Server.empty 204)) (Net.noListen env.net))
    ]


{-| A request to `/slow` given up, by a timeout of its fiber and by the
request's own: its handler hears of each.
-}
interrupted : Env -> Net -> Int -> Queue String -> Task String ()
interrupted env net port_ heard =
    let
        slow =
            Client.get ("http://127.0.0.1:" ++ String.fromInt port_ ++ "/slow")

        heardTwice =
            Task.map2 (\a b -> a ++ ", " ++ b) (Queue.take heard) (Queue.take heard)
    in
    Nomi.Task.timeout 300 (Client.send slow net)
        |> Task.map (\given -> "fiber timed out: " ++ Debug.toString (Maybe.map .status given))
        |> Task.onError (\error -> Task.succeed (Debug.toString error))
        |> Task.andThen (\line -> heardTwice |> Task.map (\twice -> line ++ "; heard " ++ twice))
        |> Task.andThen (\line -> say env line)
        |> Task.andThen (\_ -> Client.send (slow |> Client.withTimeout 300) net |> Task.map (\_ -> "Ok") |> Task.onError (Debug.toString >> Task.succeed))
        |> Task.andThen (\line -> heardTwice |> Task.map (\twice -> "request timed out: " ++ line ++ "; heard " ++ twice))
        |> Task.andThen (\line -> say env line)


report : Env -> String -> List (Task String String) -> Task String ()
report env name tasks =
    Task.sequence tasks
        |> Task.andThen (\lines -> say env (name ++ "\n" ++ String.join "\n" lines))


say : Env -> String -> Task x ()
say env line =
    Stdio.printLine line env.stdout |> Task.onError (\_ -> Task.succeed ())
"#;

/// A server on the port its first argument names, or one it picks,
/// whose bodies may hold 1000 bytes, over TLS when its arguments after the
/// port are `tls`, a certificate's file and a key's: `/stream/<n>` streams
/// `n` chunks of 16 KiB, counting each as made, and `/made` says how many
/// were; `/file` serves `served.txt`; `/events` sends server-sent events,
/// each one `/emit/<data>` gives, until `/emit/end`; `/count` reads the
/// body's stream and says how many bytes it held; `/text` reads it as
/// text; any other path says hello.
const STREAMING: &str = r#"module Main exposing (main)

import Bytes
import Bytes.Encode
import Nomi exposing (Env)
import Nomi.Bytes
import Nomi.Fs as Fs
import Nomi.Http.Server as Server exposing (Request, Response)
import Nomi.Queue as Queue exposing (Queue)
import Nomi.Ref as Ref exposing (Ref)
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                defaults =
                    Server.defaults
            in
            Task.map3 (\made events secure -> ( made, events, secure )) (Ref.make 0) Queue.unbounded (tls env)
                |> Task.andThen
                    (\( made, events, secure ) ->
                        Server.listen { defaults | port_ = 0, maxBodyBytes = 1000, tls = secure, onListen = listening env } (handle env made events) env.net
                            |> Task.mapError Server.errorToString
                    )


tls : Env -> Task String (Maybe { certificate : String, key : String })
tls env =
    case env.args of
        [ _, "tls", certificate, key ] ->
            Task.map2 (\c k -> Just { certificate = c, key = k }) (Fs.read certificate env.fs) (Fs.read key env.fs)
                |> Task.mapError Fs.errorToString

        _ ->
            Task.succeed Nothing


listening : Env -> Int -> Task x ()
listening env port_ =
    Stdio.printLine ("listening on " ++ String.fromInt port_) env.stdout
        |> Task.onError (\_ -> Task.succeed ())


handle : Env -> Ref Int -> Queue String -> Request -> Task Never Response
handle env made events req =
    case Server.path req of
        [ "stream", n ] ->
            let
                chunk =
                    Bytes.Encode.encode (Bytes.Encode.sequence (List.repeat 4096 (Bytes.Encode.unsignedInt32 Bytes.BE 0x61626364)))

                count =
                    String.toInt n |> Maybe.withDefault 0
            in
            Ref.make 0
                |> Task.andThen
                    (\given ->
                        Stream.reader
                            (Ref.modify (\i -> ( i, i + 1 )) given
                                |> Task.andThen
                                    (\i ->
                                        if i < count then
                                            Ref.update ((+) 1) made |> Task.map (\_ -> Just chunk)

                                        else
                                            Task.succeed Nothing
                                    )
                            )
                    )
                |> Task.map (Server.stream 200)

        [ "made" ] ->
            Ref.get made |> Task.map (String.fromInt >> Server.text 200)

        [ "file" ] ->
            Server.file "served.txt" env.fs
                |> Task.onError (\error -> Task.succeed (Server.text 404 (Fs.errorToString error)))

        [ "events" ] ->
            Stream.reader
                (Queue.take events
                    |> Task.map
                        (\data ->
                            if data == "end" then
                                Nothing

                            else
                                Just (Server.event { name = Just "news", id = Nothing, data = data })
                        )
                )
                |> Task.map Server.events

        [ "emit", data ] ->
            Queue.offer data events |> Task.map (\_ -> Server.empty 204)

        [ "count" ] ->
            countBytes 0 (Server.body req)
                |> Task.map (String.fromInt >> Server.text 200)
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        [ "text" ] ->
            Server.bodyText req
                |> Task.map (Server.text 200)
                |> Task.onError (\error -> Task.succeed (Server.text 400 (Debug.toString error)))

        _ ->
            Task.succeed (Server.text 200 "hello")


countBytes : Int -> Stream.Reader -> Task Stream.Error Int
countBytes size from =
    Stream.read from
        |> Task.andThen
            (\next ->
                case next of
                    Just chunk ->
                        countBytes (size + Nomi.Bytes.length chunk) from

                    Nothing ->
                        Task.succeed size
            )
"#;

/// A server serving on the port it picked, stopped when dropped.
struct Serving {
    port: u16,
    server: std::process::Child,
    /// The lines the server prints on standard output.
    said: mpsc::Receiver<String>,
    /// The project the server was built in, and runs in.
    root: PathBuf,
    _work: tempfile::TempDir,
}

impl Serving {
    fn start() -> Serving {
        Serving::of(SERVER, &[])
    }

    /// `program` built and serving on port 0, with `args` after it, once
    /// it says which port it took.
    fn of(program: &str, args: &[&str]) -> Serving {
        Serving::prepared(program, args, |_| {})
    }

    /// As `of`, once `prepare` has had the project's directory.
    fn prepared(program: &str, args: &[&str], prepare: impl FnOnce(&Path)) -> Serving {
        let (work, root) = built(program);
        prepare(&root);
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
            root,
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

    fn root(&self) -> &Path {
        &self.root
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

#[test]
fn a_slow_client_holds_back_a_streamed_response() {
    nomi_testkit::require_bun();
    let serving = Serving::of(STREAMING, &[]);
    let mut slow = serving.connect();
    slow.write_all(&http::closing("GET", "/stream/2000", "", b""))
        .unwrap();
    // The client reads nothing for a while: the server makes no more than
    // the socket's buffers hold, far from 2000 chunks of 16 KiB.
    std::thread::sleep(Duration::from_millis(500));
    let made: usize = serving
        .send(&http::closing("GET", "/made", "", b""))
        .text()
        .parse()
        .unwrap();
    assert!(
        made > 0 && made < 500,
        "{made} chunks made before the client read"
    );
    let sent = http::read(slow).unwrap();
    assert_eq!(sent.status, 200);
    assert_eq!(sent.body.len(), 2000 * 16384);
    assert!(sent.body.chunks(4).all(|word| word == b"abcd"));
    let made = serving.send(&http::closing("GET", "/made", "", b"")).text();
    assert_eq!(made, "2000");
}

#[test]
fn a_file_is_served_streamed_with_its_type_and_size() {
    nomi_testkit::require_bun();
    let serving = Serving::of(STREAMING, &[]);
    let missing = serving.send(&http::closing("GET", "/file", "", b""));
    assert_eq!(
        (missing.status, missing.text().as_str()),
        (404, "served.txt: no such file or directory")
    );
    let text = "line\n".repeat(100_000);
    fs::write(serving.root().join("served.txt"), &text).unwrap();
    let sent = serving.send(&http::closing("GET", "/file", "", b""));
    assert_eq!(sent.status, 200);
    assert_eq!(
        sent.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        sent.header("content-length"),
        Some("500000"),
        "{:?}",
        sent.headers
    );
    assert_eq!(sent.text(), text);
}

#[test]
fn server_sent_events_arrive_as_they_are_sent() {
    nomi_testkit::require_bun();
    let serving = Serving::of(STREAMING, &[]);
    let (status, lines) = http::events(serving.port, "/events").unwrap();
    assert_eq!(status, 200);
    for data in ["first", "second"] {
        let emitted = serving.send(&http::closing("POST", &format!("/emit/{data}"), "", b""));
        assert_eq!(emitted.status, 204);
        // The event arrives before the next is given.
        let arrived = nomi_testkit::process::expect(&lines, &format!("data: {data}"));
        assert!(arrived.contains(&"event: news".to_owned()), "{arrived:?}");
    }
    serving.send(&http::closing("POST", "/emit/end", "", b""));
}

#[test]
fn a_body_past_its_limit_fails_with_too_large_and_text_refuses_bad_utf_8() {
    nomi_testkit::require_bun();
    let serving = Serving::of(STREAMING, &[]);
    let counted = serving.send(&http::closing("POST", "/count", "", &[b'x'; 1000]));
    assert_eq!((counted.status, counted.text().as_str()), (200, "1000"));
    let said = serving.send(&http::closing("POST", "/count", "", &[b'x'; 1001]));
    assert_eq!((said.status, said.text().as_str()), (400, "TooLarge"));
    // With no length given, the stream fails once it passes the limit.
    let mut stream = serving.connect();
    stream
        .write_all(b"POST /count HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n")
        .unwrap();
    for _ in 0..3 {
        stream.write_all(b"190\r\n").unwrap();
        stream.write_all(&[b'x'; 400]).unwrap();
        stream.write_all(b"\r\n").unwrap();
    }
    stream.write_all(b"0\r\n\r\n").unwrap();
    let said = http::read(stream).unwrap();
    assert_eq!((said.status, said.text().as_str()), (400, "TooLarge"));
    let text = serving.send(&http::closing("POST", "/text", "", b"ok \xff"));
    assert_eq!((text.status, text.text().as_str()), (400, "NotUtf8"));
}

#[test]
fn a_server_answers_over_tls_with_a_certificate_given() {
    nomi_testkit::require_bun();
    let dir = tempfile::tempdir().unwrap();
    let (certificate, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    let made = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
        ])
        .args(["-days", "1", "-subj", "/CN=localhost", "-keyout"])
        .arg(&key)
        .arg("-out")
        .arg(&certificate)
        .output()
        .expect("openssl makes the test's certificate");
    assert!(made.status.success(), "{made:?}");
    let serving = Serving::prepared(STREAMING, &["tls", "cert.pem", "key.pem"], |root| {
        fs::copy(&certificate, root.join("cert.pem")).unwrap();
        fs::copy(&key, root.join("key.pem")).unwrap();
    });
    let fetched = bun()
        .args(["-e", &format!(
            "const r = await fetch('https://127.0.0.1:{}/', {{ tls: {{ rejectUnauthorized: false }} }}); console.log(r.status, await r.text());",
            serving.port
        )])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&fetched.stdout),
        "200 hello\n",
        "{fetched:?}"
    );
    // Plain HTTP is refused.
    let plain =
        std::panic::catch_unwind(|| serving.send(&http::closing("GET", "/", "", b"")).status);
    assert!(plain.is_err() || plain.unwrap() != 200);
}

#[test]
fn a_client_gets_the_same_responses_from_a_socket_and_a_loopback_within_its_network() {
    nomi_testkit::require_bun();
    let (_work, root) = built(CLIENT);
    let ran = bun()
        .arg("dist/main.mjs")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(ran.status.success(), "{ran:?}");
    let out = String::from_utf8_lossy(&ran.stdout).into_owned();
    let exchanges = r#"get: 200 [("content-type","text/plain; charset=utf-8")] "hello"
post text: 200 [("content-type","text/plain; charset=utf-8"),("x-method","Post")] "posted"
put json: 200 [("content-type","text/plain; charset=utf-8"),("x-method","Put")] "[1,2]"
json: 201 [("content-type","application/json")] "{\"n\":1}"
cookies: 204 [("set-cookie","a=1"),("set-cookie","b=2")] ""
redirect: 201 [("content-type","application/json")] "{\"n\":1}"
see other: 200 [("content-type","text/plain; charset=utf-8"),("x-method","Get")] ""
loop: TooManyRedirects
stream: 200 [] "streamed body"
missing: 404 [("content-type","text/plain; charset=utf-8")] "not found"
bad url: BadUrl "not a url"
refused: Unreachable "#;
    let (socket, rest) = out.split_once("loopback\n").expect(&out);
    let (loopback, rest) = rest.split_once("narrowed\n").expect(&out);
    let socket = socket.strip_prefix("socket\n").expect(&out);
    assert!(socket.starts_with(exchanges), "{socket}");
    assert!(loopback.starts_with(exchanges), "{loopback}");
    // Nothing listens at port 1 on either, each saying so its own way.
    assert!(
        loopback.ends_with("refused: Unreachable \"127.0.0.1:1: connection refused\"\n"),
        "{loopback}"
    );
    assert_eq!(
        rest,
        r#"only example.com: NotAllowed "127.0.0.1"
only 127.0.0.1: Ok
redirected to localhost: NotAllowed "localhost"
narrowed twice: NotAllowed "127.0.0.1"
no listen: PermissionDenied { host = "127.0.0.1", port_ = 8080 }
fiber timed out: Nothing; heard started, finalized
request timed out: Timeout; heard started, finalized
fiber timed out: Nothing; heard started, finalized
request timed out: Timeout; heard started, finalized
"#
    );
}
