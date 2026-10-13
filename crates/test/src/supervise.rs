//! The workers that run the tests, and what they report
//! (`Supervisor.run`, plan 10.4): each a Node process running the
//! runner's program, talking to the supervisor in lines of JSON; their
//! results printed in order as the report asks.
//!
//! elm-test listens on a pipe; nomi on a loopback TCP port, as Rust's
//! standard library has no Windows pipes. With one worker elm-test runs
//! the program in its own process, which nomi's driver stands in for.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nomi_engine::console::Console;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::generate::Report;
use crate::junit::{self, Json};
use crate::runner::DRIVER;

/// The elm-test whose behaviour nomi's is, which the console's header
/// names.
pub const ELM_TEST_VERSION: &str = "0.19.3-0";

/// What a supervisor is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Supervise {
    pub report: Report,
    pub processes: usize,
    /// Whether elm-test's own messages are colored (`chalk`).
    pub color: bool,
}

/// What reaches the supervisor, in the order it comes.
enum Event {
    Connected(usize, TcpStream),
    Line(usize, String),
    Out(String),
    Err(String),
}

/// A result as a worker sent it: its text, which the JSON report prints
/// as it is, and its value.
struct Sent {
    text: String,
    value: Value,
}

impl Sent {
    fn new(raw: &RawValue) -> Sent {
        Sent {
            text: raw.get().to_owned(),
            value: serde_json::from_str(raw.get()).unwrap_or(Value::Null),
        }
    }
}

/// A worker's message, its results and message kept as sent.
#[derive(serde::Deserialize)]
struct Message<'a> {
    #[serde(rename = "type")]
    kind: String,
    #[serde(borrow, default)]
    results: Option<HashMap<String, &'a RawValue>>,
    #[serde(borrow, default)]
    message: Option<&'a RawValue>,
    #[serde(rename = "testCount", default)]
    test_count: Option<usize>,
    #[serde(rename = "exitCode", default)]
    exit_code: Option<i32>,
}

/// Runs the tests of the program at `program` as `supervise` asks,
/// printing to `console`; the exit code elm-test's would be.
pub fn run(console: &mut dyn Console, program: &Path, supervise: Supervise) -> io::Result<i32> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let (send, events) = mpsc::channel();
    let accepting = accept(listener, send.clone());
    let mut workers = Vec::new();
    for index in 0..supervise.processes.max(1) {
        let mut command = Command::new("node");
        if supervise.processes == 1 {
            command.arg("-e").arg(DRIVER);
        }
        command
            .arg(program)
            .arg(index.to_string())
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let readers = [
            forward(child.stdout.take(), send.clone(), Event::Out),
            forward(child.stderr.take(), send.clone(), Event::Err),
        ];
        workers.push(Worker {
            child,
            readers: readers.into_iter().flatten().collect(),
            exited: false,
        });
    }
    drop(send);
    let mut state = State::new(supervise);
    let code = state.supervise(console, &events, &mut workers);
    for worker in &mut workers {
        let _ = worker.child.kill();
        let _ = worker.child.wait();
    }
    // The listener waits for a connection: this one, which it drops as
    // nothing receives it any more.
    drop(events);
    let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, port));
    let _ = accepting.join();
    Ok(code)
}

struct Worker {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    exited: bool,
}

/// Accepts the workers' connections, sending each and its lines.
fn accept(listener: TcpListener, send: Sender<Event>) -> JoinHandle<()> {
    thread::spawn(move || {
        for (id, stream) in listener.incoming().enumerate() {
            let Ok(stream) = stream else { continue };
            let Ok(reading) = stream.try_clone() else {
                continue;
            };
            if send.send(Event::Connected(id, stream)).is_err() {
                return;
            }
            let send = send.clone();
            thread::spawn(move || {
                for line in BufReader::new(reading).lines() {
                    let Ok(line) = line else { return };
                    if send.send(Event::Line(id, line)).is_err() {
                        return;
                    }
                }
            });
        }
    })
}

/// Sends what `from` prints, a line at a time.
fn forward(
    from: Option<impl Read + Send + 'static>,
    send: Sender<Event>,
    event: fn(String) -> Event,
) -> Option<JoinHandle<()>> {
    let from = from?;
    Some(thread::spawn(move || {
        let mut from = BufReader::new(from);
        let mut line = Vec::new();
        while from.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            let text = String::from_utf8_lossy(&line).into_owned();
            if send.send(event(text)).is_err() {
                return;
            }
            line.clear();
        }
    }))
}

/// What the supervisor knows of the run.
struct State {
    supervise: Supervise,
    /// The next result to print, once `BEGIN` has come.
    next: Option<usize>,
    tests: Option<usize>,
    finished: usize,
    /// Each result by its index.
    results: HashMap<usize, Sent>,
    /// The indices in the order their results came, which JUnit lists.
    order: Vec<usize>,
    failures: usize,
    todos: Vec<Value>,
    started: Instant,
    connections: HashMap<usize, TcpStream>,
}

impl State {
    fn new(supervise: Supervise) -> State {
        State {
            supervise,
            next: None,
            tests: None,
            finished: 0,
            results: HashMap::new(),
            order: Vec::new(),
            failures: 0,
            todos: Vec::new(),
            started: Instant::now(),
            connections: HashMap::new(),
        }
    }

    fn supervise(
        &mut self,
        console: &mut dyn Console,
        events: &Receiver<Event>,
        workers: &mut [Worker],
    ) -> i32 {
        loop {
            match events.recv_timeout(Duration::from_millis(20)) {
                Ok(event) => {
                    if let Some(code) = self.event(console, event) {
                        return code;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return 1,
            }
            if let Some(code) = self.exits(console, events, workers) {
                return code;
            }
        }
    }

    /// What the workers that ended mean: a failure among several is a
    /// runtime exception elm-test reports, and once all have ended there
    /// is nothing left to wait for.
    fn exits(
        &mut self,
        console: &mut dyn Console,
        events: &Receiver<Event>,
        workers: &mut [Worker],
    ) -> Option<i32> {
        for worker in workers.iter_mut().filter(|w| !w.exited) {
            let Ok(Some(status)) = worker.child.try_wait() else {
                continue;
            };
            worker.exited = true;
            // What it printed and sent comes first.
            for reader in worker.readers.drain(..) {
                let _ = reader.join();
            }
            while let Ok(event) = events.try_recv() {
                if let Some(code) = self.event(console, event) {
                    return Some(code);
                }
            }
            if !status.success() {
                if self.supervise.processes > 1 {
                    console.err(&format!(
                        "{}\n",
                        self.red(
                            "\n\nThere was an unexpected runtime exception while running tests\n\n"
                        )
                    ));
                }
                return Some(1);
            }
        }
        workers.iter().all(|w| w.exited).then_some(1)
    }

    fn red(&self, text: &str) -> String {
        if self.supervise.color {
            format!("\x1b[31m{text}\x1b[39m")
        } else {
            text.to_owned()
        }
    }

    fn event(&mut self, console: &mut dyn Console, event: Event) -> Option<i32> {
        match event {
            Event::Connected(id, stream) => {
                let _ = stream.set_nodelay(true);
                self.connections.insert(id, stream);
                None
            }
            Event::Out(text) => {
                console.out(&text);
                None
            }
            Event::Err(text) => {
                console.err(&text);
                None
            }
            Event::Line(id, line) => self.response(console, id, &line),
        }
    }

    /// `handleResponse`.
    fn response(&mut self, console: &mut dyn Console, id: usize, line: &str) -> Option<i32> {
        let Ok(message) = serde_json::from_str::<Message<'_>>(line) else {
            console.err(&format!("Unrecognized message from worker: {line}\n"));
            return Some(1);
        };
        match message.kind.as_str() {
            "FINISHED" => {
                self.results(console, message.results);
                self.finished += 1;
                if self.finished == self.supervise.processes || self.supervise.processes == 1 {
                    let summary = json!({
                        "type": "SUMMARY",
                        "duration": self.started.elapsed().as_millis() as u64,
                        "failures": self.failures,
                        "todos": self.todos,
                    });
                    if let Some(stream) = self.connections.get_mut(&id) {
                        let _ = stream.write_all(summary.to_string().as_bytes());
                    }
                }
                None
            }
            "SUMMARY" => {
                self.flush(console);
                let sent = message.message.map(Sent::new);
                if message.exit_code == Some(1) {
                    let text = match sent.as_ref().map(|s| &s.value) {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "undefined".to_owned(),
                    };
                    console.err(&format!("{text}\n"));
                } else {
                    self.print(console, sent.as_ref());
                    if self.supervise.report == Report::JUnit
                        && let Some(sent) = &sent
                    {
                        console.out(&format!("{}\n", self.junit(sent)));
                    }
                }
                Some(message.exit_code.unwrap_or(1))
            }
            "BEGIN" => {
                self.tests = message.test_count;
                if let Report::Console { .. } = self.supervise.report {
                    let headline = format!("elm-test {ELM_TEST_VERSION}");
                    let bar = "-".repeat(headline.len());
                    console.out(&format!("\n{headline}\n{bar}\n\n"));
                }
                self.print(console, message.message.map(Sent::new).as_ref());
                self.next = Some(0);
                self.flush(console);
                None
            }
            "RESULTS" => {
                self.results(console, message.results);
                None
            }
            "ERROR" => {
                let text = message.message.map(Sent::new).map(|s| match s.value {
                    Value::String(text) => text,
                    other => other.to_string(),
                });
                console.err(&format!("Error: {}\n", text.unwrap_or_default()));
                Some(1)
            }
            other => {
                console.err(&format!("Unrecognized message from worker:{other}\n"));
                Some(1)
            }
        }
    }

    /// `handleResults`: each result kept, counted as the report counts
    /// them, and those due printed.
    fn results(&mut self, console: &mut dyn Console, results: Option<HashMap<String, &RawValue>>) {
        // Index order, as JavaScript lists an object's integer keys.
        let mut results: Vec<(usize, &RawValue)> = (results.unwrap_or_default().into_iter())
            .filter_map(|(index, raw)| Some((index.parse().ok()?, raw)))
            .collect();
        results.sort_by_key(|(index, _)| *index);
        for (index, raw) in results {
            let result = Sent::new(raw);
            let status = result.value["status"].as_str();
            match self.supervise.report {
                Report::Console { .. } => match status {
                    Some("todo") => self.todos.push(result.value.clone()),
                    Some("fail") => self.failures += 1,
                    _ => {}
                },
                Report::JUnit => {
                    if result.value.get("failure").is_some() {
                        self.failures += 1;
                    }
                }
                Report::Json => match status {
                    Some("fail") => self.failures += 1,
                    Some("todo") => self.todos.push(json!({
                        "labels": result.value["labels"],
                        "todo": result.value["failures"][0],
                    })),
                    _ => {}
                },
            }
            if self.results.insert(index, result).is_none() {
                self.order.push(index);
            }
        }
        self.flush(console);
    }

    /// `flushResults`: the results due, in order, while there are.
    fn flush(&mut self, console: &mut dyn Console) {
        let (Some(mut next), Some(tests)) = (self.next, self.tests) else {
            return;
        };
        while next < tests {
            let Some(result) = self.results.get(&next) else {
                break;
            };
            print(console, self.supervise.report, Some(result));
            next += 1;
        }
        self.next = Some(next);
    }

    /// The JUnit report: the summary's XML with every result, in the
    /// order they came.
    fn junit(&self, summary: &Sent) -> String {
        let parse = |text: &str| serde_json::from_str::<Json>(text).unwrap_or(Json::Null);
        let results = (self.order.iter())
            .filter_map(|index| self.results.get(index))
            .map(|result| parse(&result.text))
            .collect();
        junit::report(parse(&summary.text), results)
    }

    fn print(&self, console: &mut dyn Console, result: Option<&Sent>) {
        print(console, self.supervise.report, result);
    }
}

/// `printResult`.
fn print(console: &mut dyn Console, report: Report, result: Option<&Sent>) {
    match report {
        Report::Json => {
            let text = result.map_or("undefined", |r| r.text.as_str());
            console.out(&format!("{text}\n"));
        }
        Report::JUnit => {}
        Report::Console { .. } => {
            let Some(result) = result else { return };
            let value = &result.value;
            let field = match (value["type"].as_str(), value["status"].as_str()) {
                (Some("begin"), _) => "output",
                (Some("complete"), Some("pass")) => "distributionReport",
                (Some("complete"), Some("fail")) => "failure",
                (Some("summary"), _) => "summary",
                _ => return,
            };
            let text = match value.get(field) {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => return,
            };
            console.out(&format!("{}\n", windows_safe(&text)));
        }
    }
}

/// `makeWindowsSafe`: on Windows, the characters its consoles may lack
/// replaced. elm-test's second pattern is the six characters in a row,
/// which reporters never print, so it never applies.
fn windows_safe(text: &str) -> String {
    if !cfg!(windows) {
        return text.to_owned();
    }
    text.replace(['↓', '✗', '►'], ">")
        .replace("╵│╷╹┃╻", "|")
        .replace('═', "=")
        .replace('▔', "-")
        .replace('✔', "√")
}

/// elm-test's default seed: `Math.floor(Math.random() * 407199254740991) +
/// 1000`.
pub fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    hasher.finish() % 407_199_254_740_991 + 1000
}

/// elm-test's default worker count: the logical cores.
pub fn default_workers() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}
