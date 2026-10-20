//! The example todo server, `examples/todo-server`, built from a copy and
//! run on the tests' Bun binary, which `NOMI_TEST_BUN` may make the
//! oldest nomi supports, on the port it picks and says it took: SIGTERM lets a response in flight complete, then the
//! todos are written and the process exits within its deadline; a client
//! that disconnects interrupts its request; a handler that dies answers
//! 500 and writes one line; and thousands of requests, some cut off, leave
//! no fiber, handle or memory behind. Its own effectful tests pass.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

use nomi_testkit::http;
use nomi_testkit::process::{Group, Signal};
use website::offline;

/// Loaded before the server with `--preload`: on SIGUSR2 it writes a line
/// to stderr, `probe <fibers> <heap> <handles>`, where `fibers` counts the
/// runtime's `Fiber` objects alive in a heap snapshot, `heap` is the heap
/// in use after a full collection, and `handles` counts the process's
/// open descriptors. The server itself carries nothing for it.
const PROBE: &str = r#"import { readdirSync } from "node:fs";

// The snapshot is counted in a function of its own, so nothing of it is
// reachable once the count is back. Its nodes are four numbers each, the
// third the index of the node's class name.
function countFibers() {
  const { nodes, nodeClassNames } = Bun.generateHeapSnapshot();
  const fiber = nodeClassNames.indexOf("Fiber");
  let fibers = 0;
  for (let i = 0; i < nodes.length; i += 4) {
    if (nodes[i + 2] === fiber) fibers++;
  }
  return fibers;
}

// Bun keeps no list of its handles, so the process's open descriptors
// stand in for them: the listening socket, the connections and files.
process.on("SIGUSR2", () => {
  const fibers = countFibers();
  Bun.gc(true);
  const handles = readdirSync("/dev/fd").length;
  console.error(`probe ${fibers} ${process.memoryUsage().heapUsed} ${handles}`);
});
"#;

/// A copy of the example in a new directory, without what running it in
/// place leaves behind.
fn example() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/todo-server");
    nomi_testkit::copy_tree(&example, &root, |path| {
        !["elm-stuff", "data", "dist"]
            .iter()
            .any(|dir| path.starts_with(dir))
    })
    .unwrap();
    (work, root)
}

/// nomi in `root`, offline with `cache`, with `args`.
fn nomi(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let mut nomi = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    nomi.current_dir(root)
        .env("PATH", nomi_testkit::path_with_bun())
        .args(args);
    nomi
}

/// The example, as copied into `root`, built into `dist/main.mjs`.
fn build(root: &Path) {
    let cache = website::fixture_cache();
    let built = nomi(
        cache.path(),
        root,
        &["build", "src/Main.elm", "--output=dist/main.mjs"],
    )
    .output()
    .unwrap();
    assert!(built.status.success(), "{built:?}");
}

/// The port in the line `listening on <port>` among `stderr`, which the
/// server writes once it is bound, waiting past what a build takes.
fn listening(stderr: &Receiver<String>) -> u16 {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match stderr.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if let Some(port) = line.strip_prefix("listening on ") {
                    let port = port.parse().unwrap();
                    assert_ne!(port, 0);
                    return port;
                }
            }
            Err(e) => panic!("the server didn't say where it listens: {e}"),
        }
    }
}

/// A request closing its connection.
fn request(method: &str, target: &str, body: &str) -> Vec<u8> {
    http::closing(method, target, "", body.as_bytes())
}

/// The response read from `stream` to its end: its status and body.
fn response(stream: &mut TcpStream) -> (u16, String) {
    let response = http::read(stream).unwrap();
    (response.status, response.text())
}

/// The example serving on the port it picked, stopped when dropped.
struct Server {
    port: u16,
    child: Group,
    stderr: Receiver<String>,
    _work: tempfile::TempDir,
}

impl Server {
    /// The example built in `root`, run on the tests' Bun binary with
    /// the probe loaded.
    fn bun(work: tempfile::TempDir, root: &Path) -> Server {
        build(root);
        fs::write(root.join("probe.mjs"), PROBE).unwrap();
        let mut bun = Command::new(nomi_testkit::bun());
        bun.args(["--preload", "./probe.mjs", "dist/main.mjs"])
            .current_dir(root);
        Server::start(work, bun)
    }

    /// `command` serving, given `PORT=0`, on the port it says it took.
    fn start(work: tempfile::TempDir, mut command: Command) -> Server {
        let mut child = Group::spawn(
            command
                .env("PORT", "0")
                .stdout(Stdio::null())
                .stderr(Stdio::piped()),
        );
        let stderr = child.stderr();
        let mut server = Server {
            port: 0,
            child,
            stderr,
            _work: work,
        };
        server.port = listening(&server.stderr);
        server.until_in_flight(0);
        server
    }

    /// A new connection, once the server takes one.
    fn connect(&self) -> TcpStream {
        let started = Instant::now();
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", self.port)) {
                Ok(stream) => break stream,
                Err(e) if started.elapsed() > Duration::from_secs(10) => panic!("{e}"),
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    /// `request` sent on a new connection; the response's status and body.
    fn send(&self, request: &[u8]) -> (u16, String) {
        let mut stream = self.connect();
        stream.write_all(request).unwrap();
        response(&mut stream)
    }

    /// How many requests the server is answering, from `/health`.
    fn in_flight(&self) -> u32 {
        let (status, body) = self.send(&request("GET", "/health", ""));
        assert_eq!(status, 200);
        let count = body.strip_prefix(r#"{"inFlight":"#).unwrap();
        count.strip_suffix('}').unwrap().parse().unwrap()
    }

    /// Waits until the server answers `count` requests besides `/health`.
    fn until_in_flight(&self, count: u32) {
        let started = Instant::now();
        while self.in_flight() != count {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "never {count} in flight"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn signal(&self, signal: Signal) {
        self.child.signal(signal);
    }

    /// The probe's report: live fibers, heap in use and active handles.
    fn probe(&self) -> (usize, usize, String) {
        self.signal(Signal::USR2);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let line = (self.stderr)
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("the probe answers");
            if let Some(report) = line.strip_prefix("probe ") {
                let mut parts = report.splitn(3, ' ');
                let fibers = parts.next().unwrap().parse().unwrap();
                let heap = parts.next().unwrap().parse().unwrap();
                return (fibers, heap, parts.next().unwrap_or("").to_owned());
            }
        }
    }

    /// The exit code once the server exits, within 15 seconds, and the
    /// lines it wrote to stderr past the development mode's warning.
    fn exit(&mut self) -> (Option<i32>, Vec<String>) {
        let started = Instant::now();
        let child = self.child.child();
        while child.try_wait().unwrap().is_none() && started.elapsed().as_secs() < 15 {
            thread::sleep(Duration::from_millis(10));
        }
        let code = (child.try_wait().unwrap()).and_then(|status| status.code());
        if code.is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let lines = (self.stderr.iter())
            .filter(|line| !line.starts_with("Compiled in DEV mode."))
            .collect();
        (code, lines)
    }
}

#[test]
fn sigterm_lets_a_response_in_flight_complete_then_writes_the_todos_and_exits() {
    let (work, root) = example();
    let cache = website::fixture_cache();
    let run = nomi(cache.path(), &root, &["run"]);
    let mut server = Server::start(work, run);
    // The handler waits for the body, sent once SIGTERM has stopped the
    // server accepting connections.
    let body = r#"{"title": "drained"}"#;
    let mut stream = server.connect();
    let sent = request("POST", "/todos", body);
    stream.write_all(&sent[..sent.len() - body.len()]).unwrap();
    server.until_in_flight(1);
    assert!(!root.join("data/todos.json").exists());
    // To `nomi run`, which passes it on.
    server.signal(Signal::TERM);
    let signalled = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", server.port)) {
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => break,
            _ if signalled.elapsed() > Duration::from_secs(5) => panic!("still accepting"),
            _ => thread::sleep(Duration::from_millis(10)),
        }
    }
    stream.write_all(body.as_bytes()).unwrap();
    assert_eq!(response(&mut stream), (201, r#"{"id":1}"#.to_owned()));
    let (code, errors) = server.exit();
    assert_eq!(code, Some(143), "{errors:?}");
    // Well within the drain's 5 seconds, and not cut off by the deadline.
    assert!(signalled.elapsed() < Duration::from_secs(5));
    assert!(
        !errors.iter().any(|line| line.starts_with("nomi:")),
        "{errors:?}"
    );
    assert_eq!(
        fs::read_to_string(root.join("data/todos.json")).unwrap(),
        "{\n  \"1\": {\n    \"title\": \"drained\",\n    \"done\": false\n  }\n}"
    );
}

#[test]
fn a_client_that_disconnects_interrupts_its_request() {
    let (work, root) = example();
    let mut server = Server::bun(work, &root);
    let mut stream = server.connect();
    stream.write_all(&request("GET", "/quote", "")).unwrap();
    server.until_in_flight(1);
    let dropped = Instant::now();
    drop(stream);
    // Its finalizer ran: the quote, left alone, answers only after its
    // two-second timeout.
    server.until_in_flight(0);
    assert!(dropped.elapsed() < Duration::from_millis(1500));
    assert_eq!(
        server.send(&request("GET", "/todos", "")),
        (200, "{}".to_owned())
    );
    server.signal(Signal::TERM);
    assert_eq!(server.exit(), (Some(143), vec![]));
}

#[test]
fn a_handler_that_dies_answers_500_and_writes_one_line() {
    let (work, root) = example();
    let main = root.join("src/Main.elm");
    let source = fs::read_to_string(&main).unwrap();
    let route = r#"Task.succeed (Server.text 404 "not found")"#;
    assert_eq!(source.matches(route).count(), 1);
    fs::write(&main, source.replace(route, r#"Debug.todo "no route""#)).unwrap();
    let line = source.lines().position(|l| l.contains(route)).unwrap() + 1;
    let mut server = Server::bun(work, &root);
    assert_eq!(
        server.send(&request("GET", "/nowhere?x=1", "")),
        (500, "internal server error".to_owned())
    );
    assert_eq!(server.in_flight(), 0);
    assert_eq!(
        server.send(&request("GET", "/todos", "")),
        (200, "{}".to_owned())
    );
    server.signal(Signal::TERM);
    assert_eq!(
        server.exit(),
        (
            Some(143),
            vec![format!(
                "nomi: request GET /nowhere?x=1 died: Error: TODO in module `Main` on line {line} no route"
            )]
        )
    );
}

/// `count` requests from 8 clients at once: todos added, listed and
/// removed, and one in ten cut off, a slow quote or a body half sent.
fn load(port: u16, count: usize) {
    let clients: Vec<_> = (0..8)
        .map(|client| {
            thread::spawn(move || {
                let connect = || {
                    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    stream
                };
                let send = |sent: &[u8]| {
                    let mut stream = connect();
                    stream.write_all(sent).unwrap();
                    response(&mut stream)
                };
                let mut id = String::new();
                for k in (client..count).step_by(8) {
                    let pause = Duration::from_millis((k % 7) as u64);
                    match k % 20 {
                        0 => {
                            let mut stream = connect();
                            let _ = stream.write_all(&request("GET", "/quote", ""));
                            thread::sleep(pause);
                        }
                        10 => {
                            let mut stream = connect();
                            let _ = stream.write_all(b"POST /todos HTTP/1.1\r\nHost: localhost\r\nContent-Length: 40\r\n\r\n{\"title");
                            thread::sleep(pause);
                        }
                        _ if k % 3 == 0 => {
                            let (status, body) =
                                send(&request("POST", "/todos", r#"{"title": "load"}"#));
                            assert_eq!(status, 201, "{body}");
                            id = body.trim_start_matches(r#"{"id":"#).trim_end_matches('}').to_owned();
                        }
                        _ if k % 3 == 1 => {
                            let (status, _) = send(&request("DELETE", &format!("/todos/{id}"), ""));
                            assert_eq!(status, if id.is_empty() { 404 } else { 204 });
                        }
                        _ => assert_eq!(send(&request("GET", "/todos", "")).0, 200),
                    }
                }
            })
        })
        .collect();
    for client in clients {
        client.join().unwrap();
    }
}

#[test]
fn nothing_leaks_under_load() {
    let (work, root) = example();
    let mut server = Server::bun(work, &root);
    load(server.port, 500);
    server.until_in_flight(0);
    let (fibers, heap, handles) = server.probe();
    assert!(handles.parse::<usize>().is_ok_and(|n| n > 0), "{handles}");
    load(server.port, 2000);
    server.until_in_flight(0);
    // Back to what it was once closing connections have closed.
    let started = Instant::now();
    let after = loop {
        let after = server.probe();
        if (after.0, &after.2) == (fibers, &handles) || started.elapsed() > Duration::from_secs(10)
        {
            break after;
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!((after.0, &after.2), (fibers, &handles));
    // A request left behind keeps a fiber of over a kilobyte and its
    // request and socket: 2,000 of them would be several megabytes.
    assert!(
        after.1 < heap + 2 * 1024 * 1024,
        "the heap grew from {heap} to {} bytes",
        after.1
    );
    server.signal(Signal::TERM);
    let (code, errors) = server.exit();
    assert_eq!(code, Some(143));
    assert!(
        errors.iter().all(|line| line.starts_with("probe ")),
        "{errors:?}"
    );
}

#[test]
fn the_examples_own_tests_pass() {
    let (_work, root) = example();
    let cache = website::fixture_cache();
    let tested = nomi(cache.path(), &root, &["test", "--offline", "--no-color"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&tested.stdout);
    assert_eq!(tested.status.code(), Some(0), "{out}{tested:?}");
    assert!(out.contains("Passed:   11\nFailed:   0\n"), "{out}");
}
