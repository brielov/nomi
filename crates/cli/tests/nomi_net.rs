//! nomi/net on Bun's sockets, where the `net` suite can't look: a Unix
//! socket echoes as TCP does; a writer whose peer stops reading is held
//! back, by what the socket's buffers hold, rather than gather what it
//! writes in memory; and TLS, with a test certificate: echo when the
//! client trusts it, refusal when it doesn't or it isn't for the host, and
//! a connection upgraded to TLS.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use nomi_testkit::process::lines;
use website::offline;

/// A program picked by its arguments: `unix <path>` listens on a Unix
/// socket at the path, connects to it, sends a line, ends its side and
/// prints what came back and the peer; `flood <port>` connects to the
/// port and writes 1,000 chunks of 64 KiB, printing how many after each;
/// `tls` serves echo over TLS with `cert.pem` and `key.pem` and prints
/// the exchange of a client trusting the certificate, then of one
/// connecting to an address it isn't for, then of one trusting the
/// system's authorities alone; `starttls` serves in plain text until a client says
/// `STARTTLS`, then upgrades, and prints a client's exchange over it.
const PROGRAM: &str = r#"module Main exposing (main)

import Bytes
import Bytes.Encode
import Nomi
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs
import Nomi.Queue as Queue
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Nomi.Tcp as Tcp
import Nomi.Tls as Tls
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "unix", path ] ->
                    Queue.unbounded
                        |> Task.andThen (\listening -> Fiber.fork (Tcp.listenUnix { path = path, onListen = Queue.offer () listening } echo env.net |> Task.mapError Tcp.errorToString) |> Task.andThen (\_ -> Queue.take listening))
                        |> Task.andThen (\_ -> Tcp.connectUnix path env.net |> Task.mapError Tcp.errorToString)
                        |> Task.andThen
                            (\connection ->
                                Stream.write (Nomi.Bytes.fromString "over unix") (Tcp.writer connection)
                                    |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
                                    |> Task.andThen (\_ -> Stream.readAll { limit = 100 } (Tcp.reader connection))
                                    |> Task.mapError Stream.errorToString
                                    |> Task.map (\reply -> Maybe.withDefault "?" (Nomi.Bytes.toString reply) ++ " from " ++ Tcp.peer connection)
                            )
                        |> Task.andThen (\said -> Stdio.printLine said env.stdout |> Task.mapError Stream.errorToString)

                [ "flood", port_ ] ->
                    Tcp.connect { host = "127.0.0.1", port_ = String.toInt port_ |> Maybe.withDefault 0 } env.net
                        |> Task.mapError Tcp.errorToString
                        |> Task.andThen (\connection -> flood 1 connection env)

                [ "tls" ] ->
                    certificate env
                        |> Task.andThen
                            (\cert ->
                                serving (\onListen -> Tls.listen cert { host = "127.0.0.1", port_ = 0, onListen = onListen } echo env.net)
                                    |> Task.andThen
                                        (\port_ ->
                                            [ ( Tls.Certificates cert.certificate, "localhost" )
                                            , ( Tls.Certificates cert.certificate, "127.0.0.1" )
                                            , ( Tls.System, "localhost" )
                                            ]
                                                |> List.map (\( trust, host ) -> Tls.connect trust { host = host, port_ = port_ } env.net |> Task.mapError Tcp.errorToString |> Task.andThen exchange |> Task.onError Task.succeed)
                                                |> Task.sequence
                                        )
                            )
                        |> Task.andThen (\said -> Stdio.printLine (String.join "\n" said) env.stdout |> Task.mapError Stream.errorToString)

                [ "starttls" ] ->
                    certificate env
                        |> Task.andThen
                            (\cert ->
                                serving (\onListen -> Tcp.listen { host = "127.0.0.1", port_ = 0, onListen = onListen } (starting cert) env.net)
                                    |> Task.andThen (\port_ -> Tcp.connect { host = "127.0.0.1", port_ = port_ } env.net |> Task.mapError Tcp.errorToString)
                                    |> Task.andThen
                                        (\plain ->
                                            send "STARTTLS" plain
                                                |> Task.andThen (\_ -> Stream.read (Tcp.reader plain) |> Task.mapError Stream.errorToString)
                                                |> Task.andThen (\_ -> Tls.upgrade (Tls.Certificates cert.certificate) "localhost" plain |> Task.mapError Tcp.errorToString)
                                        )
                                    |> Task.andThen exchange
                            )
                        |> Task.andThen (\said -> Stdio.printLine said env.stdout |> Task.mapError Stream.errorToString)

                _ ->
                    Task.fail "unix <path> | flood <port> | tls | starttls"


certificate : Nomi.Env -> Task String Tls.Certificate
certificate env =
    Task.map2 Tls.Certificate (Fs.read "cert.pem" env.fs) (Fs.read "key.pem" env.fs)
        |> Task.mapError Fs.errorToString


{-| Runs the server `listen` makes, given what to do once it listens, on a
fiber of its own, and gives its port once it listens.
-}
serving : ((Int -> Task Never ()) -> Task Tcp.Error ()) -> Task String Int
serving listen =
    Queue.unbounded
        |> Task.andThen (\listening -> Fiber.fork (listen (\port_ -> Queue.offer port_ listening)) |> Task.andThen (\_ -> Queue.take listening))


{-| Reads `STARTTLS`, answers `GO`, and echoes over TLS from then on.
-}
starting : Tls.Certificate -> Tcp.Connection -> Task Never ()
starting cert plain =
    Stream.read (Tcp.reader plain)
        |> Task.andThen (\_ -> Stream.write (Nomi.Bytes.fromString "GO") (Tcp.writer plain))
        |> Task.mapError Stream.errorToString
        |> Task.andThen (\_ -> Tls.accept cert plain |> Task.mapError Tcp.errorToString)
        |> Task.andThen (\secure -> echo secure |> Task.mapError never)
        |> Task.onError (\_ -> Task.succeed ())


send : String -> Tcp.Connection -> Task String ()
send text connection =
    Stream.write (Nomi.Bytes.fromString text) (Tcp.writer connection) |> Task.mapError Stream.errorToString


{-| Sends a word, ends this side, and reads what comes back.
-}
exchange : Tcp.Connection -> Task String String
exchange connection =
    send "secret" connection
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection) |> Task.mapError Stream.errorToString)
        |> Task.andThen (\_ -> Stream.readAll { limit = 100 } (Tcp.reader connection) |> Task.mapError Stream.errorToString)
        |> Task.map (\reply -> "echoed " ++ Maybe.withDefault "?" (Nomi.Bytes.toString reply))


echo : Tcp.Connection -> Task Never ()
echo connection =
    Stream.pipe (Tcp.reader connection) (Tcp.writer connection)
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
        |> Task.onError (\_ -> Task.succeed ())


chunk : Bytes.Bytes
chunk =
    Bytes.Encode.encode (Bytes.Encode.sequence (List.repeat 65536 (Bytes.Encode.unsignedInt8 120)))


flood : Int -> Tcp.Connection -> Nomi.Env -> Task String ()
flood n connection env =
    if n > 1000 then
        Stream.close (Tcp.writer connection) |> Task.mapError Stream.errorToString

    else
        Stream.write chunk (Tcp.writer connection)
            |> Task.andThen (\_ -> Stdio.printLine ("wrote " ++ String.fromInt n) env.stdout)
            |> Task.mapError Stream.errorToString
            |> Task.andThen (\_ -> flood (n + 1) connection env)
"#;

/// The program built from an empty store, offline, so from the bundle.
fn built() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/net": "1.0.0"}, "indirect": {"elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), PROGRAM).unwrap();
    let cache = website::fixture_cache();
    let built = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args(["build", "src/Main.elm", "--output=dist/main.mjs"])
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    (work, root)
}

fn program(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(nomi_testkit::bun());
    command
        .arg(root.join("dist/main.mjs"))
        .args(args)
        .current_dir(root);
    command
}

#[test]
fn a_unix_socket_echoes_as_tcp_does() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let ran = program(&root, &["unix", "echo.sock"]).output().unwrap();
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(
        String::from_utf8(ran.stdout).unwrap(),
        "over unix from echo.sock\n"
    );
}

/// A certificate for `localhost` alone, signed by itself, which a client
/// trusts as its own authority, in `root`.
fn certificate(root: &Path) {
    let made = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt"])
        .args(["ec_paramgen_curve:P-256", "-nodes", "-days", "1"])
        .args(["-subj", "/CN=localhost", "-addext"])
        .args(["subjectAltName=DNS:localhost", "-keyout"])
        .arg(root.join("key.pem"))
        .arg("-out")
        .arg(root.join("cert.pem"))
        .output()
        .expect("openssl makes the test's certificate");
    assert!(made.status.success(), "{made:?}");
}

#[test]
fn tls_echoes_for_a_client_that_trusts_the_certificate_and_refuses_the_others() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    certificate(&root);
    let ran = program(&root, &["tls"]).output().unwrap();
    assert!(ran.status.success(), "{ran:?}");
    let said = String::from_utf8(ran.stdout).unwrap();
    let said: Vec<&str> = said.lines().collect();
    assert_eq!(
        said[..2],
        [
            "echoed secret",
            "127.0.0.1: the server's certificate isn't trusted for it: ERR_TLS_CERT_ALTNAME_INVALID"
        ]
    );
    assert!(
        said[2].starts_with("localhost: the server's certificate isn't trusted for it: "),
        "{said:?}"
    );
}

#[test]
fn a_connection_upgraded_to_tls_carries_what_follows() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    certificate(&root);
    let ran = program(&root, &["starttls"]).output().unwrap();
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8(ran.stdout).unwrap(), "echoed secret\n");
}

#[test]
fn a_writer_whose_peer_stops_reading_is_held_back() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let mut flooding = program(&root, &["flood", &port])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let wrote = lines(flooding.stdout.take().unwrap());
    let (mut peer, _) = listener.accept().unwrap();
    // The peer reads nothing for a while: the writer gets no further than
    // the sockets' buffers hold, far from 1,000 chunks of 64 KiB.
    std::thread::sleep(Duration::from_millis(500));
    let held: Vec<String> = wrote.try_iter().collect();
    assert!(
        !held.is_empty() && held.len() < 500,
        "{} chunks written before the peer read",
        held.len()
    );
    let mut all = Vec::new();
    peer.read_to_end(&mut all).unwrap();
    assert_eq!(all.len(), 1000 * 65536);
    assert!(flooding.wait().unwrap().success());
    assert_eq!(held.len() + wrote.iter().count(), 1000);
}
