//! nomi/test's effectful tests, bundled with nomi: `nomi test` runs each
//! one's task on Bun beside the pure tests, and every reporter reports
//! how it ended as it reports a pure test. Each test has a virtual clock
//! of its own and the fakes it was given, and what it leaves running is
//! interrupted when it ends. `elm-test` knows nothing of nomi/test, as
//! `elm` knows nothing of nomi's packages.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use website::offline;

const ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {"elm/core": "1.0.5"},
        "indirect": {}
    },
    "test-dependencies": {
        "direct": {"elm-explorations/test": "2.2.1", "nomi/core": "2.0.0", "nomi/test": "2.0.0"},
        "indirect": {"elm/bytes": "1.0.8", "elm/html": "1.0.1", "elm/json": "1.1.4", "elm/random": "1.0.0", "elm/time": "1.0.0", "elm/virtual-dom": "1.0.5"}
    }
}
"#;

/// A pure test beside effectful ones that pass, fail, fail their
/// expectation and die.
const TESTS: &str = r#"module Effects exposing (dies, expectationFails, fails, passes, pure)

import Expect
import Nomi.Fs as Fs
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Nomi.Test exposing (effect)
import Process
import Task
import Test exposing (Test, test)


passes : Test
passes =
    effect "a file reads back" <|
        \env ->
            Fs.write "a.txt" "hello" env.fs
                |> Task.andThen (\_ -> Fs.read "a.txt" env.fs)
                |> Task.mapError Fs.errorToString
                |> Task.andThen (\text -> Stdio.print text env.stdout |> Task.mapError Stream.errorToString |> Task.map (\_ -> text))
                |> Task.map (Expect.equal "hello")


fails : Test
fails =
    effect "the task fails" <|
        \_ -> Process.sleep 10 |> Task.andThen (\_ -> Task.fail "boom")


expectationFails : Test
expectationFails =
    effect "each test has a file system of its own" <|
        \env ->
            Fs.exists "a.txt" env.fs
                |> Task.mapError Fs.errorToString
                |> Task.map (Expect.equal True)


dies : Test
dies =
    effect "the task dies" <|
        \_ -> Task.succeed () |> Task.andThen (\_ -> Debug.todo "dead")


pure : Test
pure =
    test "a pure test" <|
        \() -> Expect.equal 2 (1 + 1)
"#;

/// Effectful tests of the virtual clock and the fakes: an hour's sleep
/// and three ticks of a second, with `advance`; two tests each sleeping a
/// second from 0; one that leaves a ticking daemon behind, logging each
/// tick; a program given files and arguments, whose output is read; one
/// whose stderr has what it printed there and logged; and two given input
/// and variables, each reading the input from its start.
const CLOCK: &str = r#"module Clock exposing (aFreshCopy, aLeftover, anHourAndThreeTicks, anotherFreshCopy, filesAndArgs, input, inputAgain, logged, oneClock, otherClock)

import Expect exposing (Expectation)
import Nomi exposing (Env)
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Nomi.Task
import Dict
import Nomi.Test exposing (advance, effect, effectWith, stderr, stdout, withArgs, withFiles, withStdin, withVars)
import Process
import Task exposing (Task)
import Test exposing (Test)
import Time


anHourAndThreeTicks : Test
anHourAndThreeTicks =
    effect "an hour's sleep and three ticks take no real time" <|
        \env ->
            Fiber.fork (Process.sleep 3600000 |> Task.andThen (\_ -> stamp "slept" env) |> Task.andThen (\_ -> ticks env))
                |> Task.andThen (\_ -> advance 3600000)
                |> Task.andThen (\_ -> stdout env)
                |> Task.andThen (\slept -> advance 3000 |> Task.andThen (\_ -> Task.map2 (\out now -> ( slept, out, now )) (stdout env) Time.now))
                |> Task.map
                    (Expect.equal
                        ( "slept 3600000\n"
                        , "slept 3600000\ntick 1 3601000\ntick 2 3602000\ntick 3 3603000\n"
                        , Time.millisToPosix 3603000
                        )
                    )


ticks : Env -> Task x ()
ticks env =
    List.range 1 3
        |> List.map (\i -> Process.sleep 1000 |> Task.andThen (\_ -> stamp ("tick " ++ String.fromInt i) env))
        |> Task.sequence
        |> Task.map (\_ -> ())


stamp : String -> Env -> Task x ()
stamp name env =
    Time.now |> Task.andThen (\time -> Stdio.printLine (name ++ " " ++ String.fromInt (Time.posixToMillis time)) env.stdout |> Task.onError (\_ -> Task.succeed ()))


oneClock : Test
oneClock =
    effect "a test's clock starts at 0" <| \_ -> aSecondFromZero


otherClock : Test
otherClock =
    effect "another test's clock starts at 0 too" <| \_ -> aSecondFromZero


aSecondFromZero : Task x Expectation
aSecondFromZero =
    Process.sleep 1000 |> Task.andThen (\_ -> Time.now) |> Task.map (Expect.equal (Time.millisToPosix 1000))


aLeftover : Test
aLeftover =
    effect "a test may leave a ticking daemon behind" <|
        \env ->
            Process.spawn (tickForever env |> Nomi.Task.ensure (log "leftover interrupted"))
                |> Task.andThen (\_ -> advance 2500)
                |> Task.andThen (\_ -> stdout env)
                |> Task.map (Expect.equal "tick\ntick\n")


tickForever : Env -> Task x ()
tickForever env =
    Process.sleep 1000
        |> Task.andThen (\_ -> log "leftover tick")
        |> Task.andThen (\_ -> Stdio.printLine "tick" env.stdout |> Task.onError (\_ -> Task.succeed ()))
        |> Task.andThen (\_ -> tickForever env)


log : String -> Task x ()
log line =
    Task.succeed () |> Task.map (\_ -> Debug.log line ())


given : Env
given =
    Nomi.Test.env |> withFiles [ ( "data/greeting.txt", "hello" ) ] |> withArgs [ "data/greeting.txt" ]


filesAndArgs : Test
filesAndArgs =
    effectWith given "files and arguments reach the program, and stdout has what it printed" <|
        \env ->
            greet env
                |> Task.andThen (\_ -> stdout env)
                |> Task.map (Expect.equal "hello, world\n")


greet : Env -> Task String ()
greet env =
    case env.args of
        [ path ] ->
            Fs.read path env.fs
                |> Task.mapError Fs.errorToString
                |> Task.andThen (\text -> Stdio.printLine (text ++ ", world") env.stdout |> Task.mapError Stream.errorToString)

        _ ->
            Task.fail "no path"


logged : Test
logged =
    effect "stderr has what was printed to it and logged" <|
        \env ->
            Stdio.printLine "printed" env.stderr
                |> Task.mapError Stream.errorToString
                |> Task.andThen (\_ -> Fiber.fork (Nomi.log "logged in a fiber") |> Task.andThen Fiber.join)
                |> Task.andThen (\_ -> Task.map2 Tuple.pair (stdout env) (stderr env))
                |> Task.map (Expect.equal ( "", "printed\nlogged in a fiber\n" ))


aFreshCopy : Test
aFreshCopy =
    effectWith given "a test writes into its own copy of the files" <|
        \env ->
            Fs.write "data/new.txt" "new" env.fs
                |> Task.andThen
                    (\_ ->
                        Task.map2 Tuple.pair
                            (Fs.exists "data/new.txt" (withFiles [ ( "more.txt", "more" ) ] env).fs)
                            (Fs.exists "more.txt" env.fs)
                    )
                |> Task.mapError Fs.errorToString
                |> Task.map (Expect.equal ( True, False ))


typed : Env
typed =
    Nomi.Test.env |> withVars [ ( "A", "1" ), ( "B", "2" ) ] |> withVars [ ( "B", "3" ) ] |> withStdin "one\ntwo"


input : Test
input =
    effectWith typed "a test reads the input and variables it is given" <| readTyped


inputAgain : Test
inputAgain =
    effectWith typed "another test reads the same input from its start" <| readTyped


readTyped : Env -> Task String Expectation
readTyped env =
    Task.map3 (\a b c -> ( [ a, b, c ], Dict.toList env.vars ))
        (Stdio.line env.stdin)
        (Stdio.line env.stdin)
        (Stdio.line env.stdin)
        |> Task.mapError Stream.errorToString
        |> Task.map (Expect.equal ( [ Just "one", Just "two", Nothing ], [ ( "A", "1" ), ( "B", "3" ) ] ))


anotherFreshCopy : Test
anotherFreshCopy =
    effectWith given "another test's copy has only the files given" <|
        \env ->
            Task.map2 Tuple.pair (Fs.exists "data/greeting.txt" env.fs) (Fs.exists "data/new.txt" env.fs)
                |> Task.mapError Fs.errorToString
                |> Task.map (Expect.equal ( True, False ))
"#;

/// nomi/core's concurrent tasks and fibers in an effectful test, printing
/// how each ends. What it waits for to lose takes an hour, so it ends at
/// once only on the virtual clock.
const CONCURRENT: &str = r#"module Concurrent exposing (concurrent)

import Expect
import Nomi.Fiber as Fiber exposing (Exit(..))
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Nomi.Task
import Nomi.Test exposing (effect, stdout)
import Process
import Task exposing (Task)
import Test exposing (Test)


concurrent : Test
concurrent =
    effect "concurrent tasks and fibers end and interrupt as documented" <|
        \env ->
            Task.sequence (checks (\line -> Stdio.printLine line env.stdout |> Task.onError (\_ -> Task.succeed ())))
                |> Task.andThen (\_ -> stdout env)
                |> Task.map (\out -> Expect.equal expected (String.lines (String.trimRight out)))


expected : List String
expected =
    [ "both's other finalized"
    , "both: Err \"first\""
    , "both: Ok (1,2)"
    , "all: Ok [1,2,3]"
    , "race's loser finalized"
    , "race: Ok \"fast\""
    , "timed out finalized"
    , "timeout: Ok Nothing"
    , "timeout: Ok (Just \"quick\")"
    , "region starts"
    , "interrupting"
    , "region ends"
    , "uninterruptible: Ok Interrupted"
    , "await: Ok (Success 1)"
    , "await: Ok (Failure \"no\")"
    , "await: Ok \"Defect, Error: TODO in module `Concurrent`\""
    , "interrupting"
    , "awaited finalized"
    , "await: Ok Interrupted"
    , "forkDaemon: Ok (Success \"daemon\")"
    , "fork: Ok Interrupted"
    , "acquired a"
    , "released a"
    , "acquireRelease: Ok 1"
    , "acquired b"
    , "released b"
    , "acquireRelease: Err \"no\""
    , "acquired c"
    , "interrupting"
    , "using c finalized"
    , "released c"
    , "acquireRelease: Ok Interrupted"
    ]


type alias Say =
    String -> Task Never ()


checks : Say -> List (Task String ())
checks say =
    [ show say "both" (Nomi.Task.both (after 10 (Task.fail "first")) (hour say "both's other"))
    , show say "both" (Nomi.Task.both (after 20 (Task.succeed 1)) (after 10 (Task.succeed 2)))
    , show say "all" (Nomi.Task.all [ after 30 (Task.succeed 1), after 10 (Task.succeed 2), after 20 (Task.succeed 3) ])
    , show say "race" (Nomi.Task.race (after 10 (Task.succeed "fast")) (hour say "race's loser"))
    , show say "timeout" (Nomi.Task.timeout 50 (hour say "timed out"))
    , show say "timeout" (Nomi.Task.timeout 50 (after 10 (Task.succeed "quick")))
    , Fiber.fork
        (say "region starts"
            |> Task.andThen (\_ -> Process.sleep 100)
            |> Task.andThen (\_ -> say "region ends")
            |> Nomi.Task.uninterruptible
        )
        |> Task.andThen (interruptAfter say 50)
        |> show say "uninterruptible"
    , Fiber.fork (Task.succeed 1) |> Task.andThen Fiber.await |> show say "await"
    , Fiber.fork (Task.fail "no") |> Task.andThen Fiber.await |> show say "await"
    , Fiber.fork (Task.succeed () |> Task.andThen (\_ -> Debug.todo "boom"))
        |> Task.andThen Fiber.await
        |> Task.map defect
        |> show say "await"
    , Fiber.fork (hour say "awaited") |> Task.andThen (interruptAfter say 10) |> show say "await"
    , Fiber.fork (Fiber.forkDaemon (after 10 (Task.succeed "daemon")))
        |> Task.andThen Fiber.join
        |> Task.andThen Fiber.await
        |> show say "forkDaemon"
    , Fiber.fork (Fiber.fork (after 10 (Task.succeed "child")))
        |> Task.andThen Fiber.join
        |> Task.andThen Fiber.await
        |> show say "fork"
    , show say "acquireRelease" (resource say "a" (Task.succeed 1))
    , show say "acquireRelease" (resource say "b" (Task.fail "no"))
    , Fiber.fork (resource say "c" (hour say "using c")) |> Task.andThen (interruptAfter say 10) |> show say "acquireRelease"
    ]


after : Float -> Task x a -> Task x a
after ms task =
    Process.sleep ms |> Task.andThen (\_ -> task)


hour : Say -> String -> Task String a
hour say name =
    after 3600000 (Task.fail "too late") |> Nomi.Task.ensure (say (name ++ " finalized"))


interruptAfter : Say -> Float -> Fiber.Fiber x a -> Task y (Exit x a)
interruptAfter say ms fiber =
    Process.sleep ms
        |> Task.andThen (\_ -> speak say "interrupting")
        |> Task.andThen (\_ -> Fiber.interrupt fiber)


resource : Say -> String -> Task String a -> Task String a
resource say name use =
    Nomi.Task.acquireRelease
        (speak say ("acquired " ++ name) |> Task.map (\_ -> name))
        (\r -> say ("released " ++ r))
        (\_ -> use)


defect : Exit x a -> String
defect exit =
    case exit of
        Defect message ->
            "Defect, " ++ String.left 34 message

        _ ->
            "not a defect"


show : Say -> String -> Task x a -> Task y ()
show say label task =
    task
        |> Task.map Ok
        |> Task.onError (Task.succeed << Err)
        |> Task.andThen (\result -> speak say (label ++ ": " ++ Debug.toString result))


speak : Say -> String -> Task x ()
speak say line =
    say line |> Task.mapError never
"#;

/// The test dependencies of `SERVING`: nomi/server, and elm/http for a
/// client.
const SERVING_ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {"elm/core": "1.0.5"},
        "indirect": {}
    },
    "test-dependencies": {
        "direct": {"elm-explorations/test": "2.2.1", "elm/http": "2.0.0", "nomi/core": "2.0.0", "nomi/server": "1.0.0", "nomi/test": "2.0.0"},
        "indirect": {"elm/bytes": "1.0.8", "elm/file": "1.0.5", "elm/html": "1.0.1", "elm/json": "1.1.4", "elm/random": "1.0.0", "elm/time": "1.0.0", "elm/virtual-dom": "1.0.5"}
    }
}
"#;

/// Handlers in effectful tests: a request made in the test reads as a
/// received one, its repeated headers by Fetch's rules; a handler that
/// dies, or gives a response Fetch won't write, answers 500 and logs to the
/// test's stderr; headers added twice are both written; a server listening
/// in a test, whose client disconnects, interrupts the request on the
/// test's runtime, so its finalizer logs there too; and one listening
/// leaves the test's clock free to move.
const SERVING: &str = r#"module Serving exposing (aDefect, aDisconnect, aRequest, aSleepBesideAServer, addedHeaders, repeatedHeaders, unwritable)

import Expect
import Http
import Nomi
import Nomi.Fiber as Fiber
import Nomi.Queue as Queue
import Nomi.Server as Server exposing (Method(..))
import Nomi.Task
import Nomi.Test exposing (effect, stderr)
import Process
import Task exposing (Task)
import Test exposing (Test)
import Time


aRequest : Test
aRequest =
    effect "a request made in a test reads as one received" <|
        \_ ->
            Task.map2 Tuple.pair
                (Server.respond echo (Server.request { method = Patch, target = "/a/b%20c/?q=1&q=2", headers = [ ( "X-A", "1" ), ( "x-a", "2" ) ], body = "hi" }))
                (Server.respond echo (Server.request { method = Get, target = "/", headers = [], body = String.repeat 1048577 "a" }))
                |> Task.map
                    (Expect.equal
                        ( { status = 200, headers = [ ( "content-type", "text/plain; charset=utf-8" ), ( "x-echo", "yes" ) ], body = "True a/b c 1 1, 2 hi" }
                        , { status = 413, headers = [], body = "" }
                        )
                    )


echo : Server.Request -> Task Never Server.Response
echo req =
    Server.body req
        |> Task.map
            (\body ->
                Server.text 200 (String.join " " [ Debug.toString (Server.method req == Patch), String.join "/" (Server.path req), Maybe.withDefault "-" (Server.query "q" req), Maybe.withDefault "-" (Server.header "X-a" req), body ])
                    |> Server.withHeader "X-Echo" "yes"
            )
        |> Task.onError (\_ -> Task.succeed (Server.empty 413))


aDefect : Test
aDefect =
    effect "a handler that dies answers 500 and logs to the test's stderr" <|
        \env ->
            Server.respond (\_ -> Task.succeed () |> Task.andThen (\_ -> Debug.todo "dead")) (Server.request { method = OtherMethod "BREW", target = "/pot?x=1", headers = [], body = "" })
                |> Task.andThen (\response -> stderr env |> Task.map (\err -> ( response, String.startsWith "nomi: request BREW /pot?x=1 died: Error: TODO in module `Serving`" err )))
                |> Task.map (Expect.equal ( { status = 500, headers = [ ( "content-type", "text/plain; charset=utf-8" ) ], body = "internal server error" }, True ))


aDisconnect : Test
aDisconnect =
    effect "a client that disconnects interrupts its request on the test's runtime" <|
        \env ->
            Queue.unbounded
                |> Task.andThen
                    (\events ->
                        let
                            defaults =
                                Server.defaults

                            config =
                                { defaults | port_ = 0, onListen = \port_ -> Queue.offer (String.fromInt port_) events }

                            handle _ =
                                Nomi.Task.acquireRelease (Queue.offer "started" events)
                                    (\_ -> Nomi.log "released" |> Task.andThen (\_ -> Queue.offer "released" events))
                                    (\_ -> Process.sleep 60000 |> Task.map (\_ -> Server.text 200 "late"))
                        in
                        Fiber.fork (Server.listen config handle env.net)
                            |> Task.andThen (\_ -> Queue.take events)
                            |> Task.andThen (\port_ -> Fiber.fork (get port_))
                            |> Task.andThen (\client -> Queue.take events |> Task.andThen (\started -> Fiber.interrupt client |> Task.map (\_ -> started)))
                            |> Task.andThen (\started -> Queue.take events |> Task.map (\released -> [ started, released ]))
                    )
                |> Task.andThen (\events -> stderr env |> Task.map (Tuple.pair events))
                |> Task.map (Expect.equal ( [ "started", "released" ], "released\n" ))


repeatedHeaders : Test
repeatedHeaders =
    effect "a request made in a test reads repeated headers by Fetch's rules" <|
        \_ ->
            let
                headers =
                    [ ( "Cookie", "a=1" ), ( "cookie", "b=2" ), ( "Content-Type", "a" ), ( "content-type", "b" ) ]

                read req =
                    Task.succeed (Server.text 200 (Debug.toString ( Server.header "cookie" req, Server.header "Content-Type" req )))
            in
            Server.respond read (Server.request { method = Get, target = "/", headers = headers, body = "" })
                |> Task.map (.body >> Expect.equal "(Just \"a=1; b=2\",Just \"a, b\")")


unwritable : Test
unwritable =
    effect "a response Fetch won't write answers 500, as listen does" <|
        \env ->
            Task.map2 Tuple.pair
                (Server.respond (\_ -> Task.succeed (Server.empty 999999)) (Server.request { method = Get, target = "/status", headers = [], body = "" }))
                (Server.respond (\_ -> Task.succeed (Server.empty 200 |> Server.withHeader "X-Bad" "a\nb")) (Server.request { method = Get, target = "/header", headers = [], body = "" }))
                |> Task.andThen (\responses -> stderr env |> Task.map (Tuple.pair responses))
                |> Task.map
                    (Expect.equal
                        ( ( { status = 500, headers = [ ( "content-type", "text/plain; charset=utf-8" ) ], body = "internal server error" }
                          , { status = 500, headers = [ ( "content-type", "text/plain; charset=utf-8" ) ], body = "internal server error" }
                          )
                        , "nomi: request GET /status died: RangeError: The status provided (999999) must be 101 or in the range of [200, 599]\nnomi: request GET /header died: TypeError: Header 'X-Bad' has invalid value\n"
                        )
                    )


addedHeaders : Test
addedHeaders =
    effect "headers added twice are both written, and one set replaces those of its name" <|
        \_ ->
            let
                cookies =
                    Server.empty 204 |> Server.addHeader "Set-Cookie" "a=1" |> Server.addHeader "X-Between" "1" |> Server.addHeader "set-cookie" "b=2"
            in
            Task.map2 Tuple.pair
                (Server.respond (\_ -> Task.succeed cookies) (Server.request { method = Get, target = "/", headers = [], body = "" }))
                (Server.respond (\_ -> Task.succeed (cookies |> Server.withHeader "SET-COOKIE" "c=3")) (Server.request { method = Get, target = "/", headers = [], body = "" }))
                |> Task.map (Tuple.mapBoth .headers .headers >> Expect.equal ( [ ( "set-cookie", "a=1" ), ( "set-cookie", "b=2" ), ( "x-between", "1" ) ], [ ( "set-cookie", "c=3" ), ( "x-between", "1" ) ] ))


aSleepBesideAServer : Test
aSleepBesideAServer =
    effect "a server listening in a test leaves the test's clock free to move" <|
        \env ->
            Queue.unbounded
                |> Task.andThen
                    (\listening ->
                        let
                            defaults =
                                Server.defaults
                        in
                        Fiber.fork (Server.listen { defaults | port_ = 0, onListen = \_ -> Queue.offer () listening } (\_ -> Task.succeed (Server.empty 204)) env.net)
                            |> Task.andThen (\_ -> Queue.take listening)
                            |> Task.andThen (\_ -> Process.sleep 1000)
                            |> Task.andThen (\_ -> Nomi.Task.timeout 1000 (Process.sleep 3600000))
                            |> Task.andThen (\timedOut -> Time.now |> Task.map (Tuple.pair timedOut))
                    )
                |> Task.map (Expect.equal ( Nothing, Time.millisToPosix 2000 ))


get : String -> Task Http.Error ()
get port_ =
    Http.task
        { method = "GET"
        , headers = []
        , url = "http://127.0.0.1:" ++ port_ ++ "/"
        , body = Http.emptyBody
        , resolver = Http.stringResolver (\_ -> Ok ())
        , timeout = Nothing
        }
"#;

/// `program` run with `command`, then `args`, in a new project with
/// `elm_json` whose tests are `modules`, each a name and its source,
/// offline with the fixture packages and nomi's own bundle.
fn run(
    program: &Path,
    elm_json: &str,
    modules: &[(&str, &str)],
    command: &[&str],
    args: &[&str],
) -> Output {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(root.join("elm.json"), elm_json).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    for (name, source) in modules {
        fs::write(root.join(format!("tests/{name}.elm")), source).unwrap();
    }
    let cache = website::fixture_cache();
    offline(program, cache.path())
        .current_dir(&root)
        .args(command)
        .args(["--offline", "--seed", "1", "--no-color"])
        .args(args)
        .output()
        .unwrap()
}

/// `nomi test` with `args` on `modules`.
fn nomi_test(modules: &[(&str, &str)], args: &[&str]) -> Output {
    run(
        Path::new(env!("CARGO_BIN_EXE_nomi")),
        ELM_JSON,
        modules,
        &["test"],
        args,
    )
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn effectful_tests_run_beside_pure_ones() {
    nomi_testkit::require_bun();
    let output = nomi_test(&[("Effects", TESTS)], &["--workers", "2"]);
    let (out, err) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(2), "{out}\n{err}");
    assert_eq!(err, "", "{out}");
    let failures = "Running 5 tests. To reproduce these results, run: elm-test --fuzz 100 --seed 1\n\n\
        ↓ Effects\n✗ the task dies\n\n    This test failed because its task died: \"Error: TODO in module `Effects` on line 42\n    \n    dead\"\n\n\n\
        ↓ Effects\n✗ each test has a file system of its own\n\n    ▼▼▼▼ \n    False\n    ╷\n    │ Expect.equal\n    ╵\n    True\n    ▲▲▲ \n\n\n\
        ↓ Effects\n✗ the task fails\n\n    boom\n\n\n\nTEST RUN FAILED\n";
    assert!(out.contains(failures), "{out}");
    assert!(out.contains("Passed:   2\nFailed:   3\n"), "{out}");
    // The test's output was kept, not printed.
    assert!(!out.contains("hello"), "{out}");
}

#[test]
fn the_json_report_has_effectful_tests_results() {
    nomi_testkit::require_bun();
    let output = nomi_test(
        &[("Effects", TESTS)],
        &["--report", "json", "--workers", "1"],
    );
    let out = text(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{out}");
    let events: Vec<serde_json::Value> = (out.lines())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completed: Vec<(String, String, Vec<String>)> = (events.iter())
        .filter(|event| event["event"] == "testCompleted")
        .map(|event| {
            let labels = event["labels"].as_array().unwrap();
            let messages = (event["failures"].as_array().unwrap().iter())
                .map(|failure| failure["message"].as_str().unwrap().to_owned())
                .collect();
            (
                labels.last().unwrap().as_str().unwrap().to_owned(),
                event["status"].as_str().unwrap().to_owned(),
                messages,
            )
        })
        .collect();
    let completed: Vec<(&str, &str, Vec<&str>)> = (completed.iter())
        .map(|(label, status, messages)| {
            let messages = messages.iter().map(String::as_str).collect();
            (label.as_str(), status.as_str(), messages)
        })
        .collect();
    assert_eq!(
        completed,
        [
            (
                "the task dies",
                "fail",
                vec![
                    "This test failed because its task died: \"Error: TODO in module `Effects` on line 42\n\ndead\""
                ]
            ),
            (
                "each test has a file system of its own",
                "fail",
                vec!["Expect.equal"]
            ),
            ("the task fails", "fail", vec!["boom"]),
            ("a file reads back", "pass", vec![]),
            ("a pure test", "pass", vec![]),
        ]
    );
    let last = events.last().unwrap();
    assert_eq!(
        (&last["event"], &last["passed"], &last["failed"]),
        (&"runComplete".into(), &"2".into(), &"3".into())
    );
}

#[test]
fn elm_test_knows_nothing_of_nomi_test() {
    nomi_testkit::require_bun();
    let (_bin, elm_test) = website::personality("elm-test");
    let output = run(&elm_test, ELM_JSON, &[("Effects", TESTS)], &[], &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        text(&output.stderr).contains("I could not find versions of the project's dependencies"),
        "{output:?}"
    );
}

/// The JSON report's completed tests: each one's last label, status and
/// duration in milliseconds. The lines that aren't JSON are the tests'
/// own output.
fn completed(out: &str) -> Vec<(String, String, u64)> {
    (out.lines())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["event"] == "testCompleted")
        .map(|event| {
            let label = event["labels"].as_array().unwrap().last().unwrap();
            (
                label.as_str().unwrap().to_owned(),
                event["status"].as_str().unwrap().to_owned(),
                event["duration"].as_str().unwrap().parse().unwrap(),
            )
        })
        .collect()
}

#[test]
fn each_effectful_test_has_a_virtual_clock_and_fakes_of_its_own() {
    nomi_testkit::require_bun();
    let output = nomi_test(&[("Clock", CLOCK)], &["--report", "json", "--workers", "1"]);
    let (out, err) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(0), "{out}\n{err}");
    let completed = completed(&out);
    assert_eq!(completed.len(), 10, "{out}");
    for (label, status, duration) in &completed {
        assert_eq!(status, "pass", "{label}: {out}");
        // An hour and three seconds of the test's clock take no real time.
        assert!(*duration < 1000, "{label} took {duration} ms");
    }
    // The daemon a test left behind ticked twice, then was interrupted.
    assert_eq!(out.matches("leftover tick").count(), 2, "{out}");
    assert_eq!(out.matches("leftover interrupted").count(), 1, "{out}");
    // What a test logs is its own, not the runner's.
    assert!(!err.contains("logged in a fiber"), "{err}");
}

#[test]
fn concurrent_tasks_and_fibers_end_and_interrupt_as_documented_on_the_virtual_clock() {
    nomi_testkit::require_bun();
    let output = nomi_test(&[("Concurrent", CONCURRENT)], &["--report", "json"]);
    let out = text(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{out}");
    let completed = completed(&out);
    assert_eq!(completed.len(), 1, "{out}");
    let (_, status, duration) = &completed[0];
    assert_eq!(status, "pass", "{out}");
    assert!(*duration < 1000, "{out}");
}

#[test]
fn handlers_are_tested_with_requests_made_in_the_test_or_a_server_listening_in_it() {
    nomi_testkit::require_bun();
    let output = run(
        Path::new(env!("CARGO_BIN_EXE_nomi")),
        SERVING_ELM_JSON,
        &[("Serving", SERVING)],
        &["test"],
        &["--report", "json"],
    );
    let (out, err) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(0), "{out}\n{err}");
    let completed = completed(&out);
    assert_eq!(completed.len(), 7, "{out}");
    for (label, status, _) in &completed {
        assert_eq!(status, "pass", "{label}: {out}");
    }
    // What the handlers logged was the tests', not the runner's.
    assert_eq!(err, "", "{out}");
}

/// A test that never ends: a server listening, which holds its worker's
/// event loop, and a wait for a message nobody sends.
const HANGING: &str = r#"module Hanging exposing (hangs)

import Nomi.Fiber as Fiber
import Nomi.Queue as Queue
import Nomi.Server as Server
import Nomi.Test exposing (effect)
import Task
import Test exposing (Test)


hangs : Test
hangs =
    effect "never ends" <|
        \env ->
            Queue.unbounded
                |> Task.andThen
                    (\never ->
                        let
                            defaults =
                                Server.defaults
                        in
                        Fiber.fork (Server.listen { defaults | port_ = 0 } (\_ -> Task.succeed (Server.empty 204)) env.net)
                            |> Task.andThen (\_ -> Queue.take never)
                    )
"#;

#[test]
fn a_worker_exits_when_its_supervisor_is_killed() {
    nomi_testkit::require_bun();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(root.join("elm.json"), SERVING_ELM_JSON).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    fs::write(root.join("tests/Hanging.elm"), HANGING).unwrap();
    let cache = website::fixture_cache();
    let mut nomi = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args([
            "test",
            "--offline",
            "--seed",
            "1",
            "--no-color",
            "--workers",
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let children = || -> Vec<String> {
        let found = Command::new("pgrep")
            .args(["-P", &nomi.id().to_string()])
            .output()
            .unwrap();
        text(&found.stdout).lines().map(str::to_owned).collect()
    };
    let started = Instant::now();
    let mut workers = children();
    while workers.len() < 2 && started.elapsed() < Duration::from_secs(60) {
        thread::sleep(Duration::from_millis(50));
        workers = children();
    }
    assert_eq!(workers.len(), 2, "{workers:?}");
    // Give the workers time to connect and start the test.
    thread::sleep(Duration::from_secs(2));
    nomi.kill().unwrap();
    nomi.wait().unwrap();
    let alive = |pid: &String| {
        let probe = Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status();
        probe.unwrap().success()
    };
    let killed = Instant::now();
    while workers.iter().any(alive) && killed.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(50));
    }
    let orphans: Vec<_> = workers.iter().filter(|pid| alive(pid)).cloned().collect();
    for pid in &orphans {
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
    assert_eq!(orphans, Vec::<String>::new());
}
