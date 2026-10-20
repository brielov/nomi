//! The workers that run the tests, and what they report
//! (`Supervisor.run`): each a Bun process running the
//! runner's program, talking to the supervisor in lines of JSON; their
//! results printed in order as the report asks.
//!
//! elm-test listens on a pipe; nomi on a loopback TCP port, as Rust's
//! standard library has no Windows pipes. With one worker elm-test runs
//! the program in its own process, which nomi's driver stands in for.

use std::borrow::Cow;
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
use crate::json::Json;
use crate::junit;
use crate::runner::DRIVER;

/// The elm-test whose behaviour nomi's is, which the console's header
/// names.
pub(crate) const ELM_TEST_VERSION: &str = "0.19.3-0";

/// What a supervisor is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Supervise {
    pub report: Report,
    /// How many workers run the tests.
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

/// A result as a worker sent it, for the streamed reports: its text,
/// which the JSON report prints as it is, and its value, with the fields
/// the reports read.
struct Sent {
    text: String,
    value: Value,
    status: Option<Status>,
    stage: Option<Stage>,
}

impl Sent {
    fn new(raw: &RawValue) -> Sent {
        let value: Value = serde_json::from_str(raw.get()).unwrap_or(Value::Null);
        let status = match value["status"].as_str() {
            Some("pass") => Some(Status::Pass),
            Some("fail") => Some(Status::Fail),
            Some("todo") => Some(Status::Todo),
            _ => None,
        };
        let stage = match value["type"].as_str() {
            Some("begin") => Some(Stage::Begin),
            Some("complete") => Some(Stage::Complete),
            Some("summary") => Some(Stage::Summary),
            _ => None,
        };
        Sent {
            text: raw.get().to_owned(),
            value,
            status,
            stage,
        }
    }
}

/// A result's `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Todo,
}

/// A console result's `type`: what it reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Begin,
    Complete,
    Summary,
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

/// A message's `type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Finished,
    Summary,
    Begin,
    Results,
    Error,
}

impl Kind {
    fn parse(text: &str) -> Option<Kind> {
        match text {
            "FINISHED" => Some(Kind::Finished),
            "SUMMARY" => Some(Kind::Summary),
            "BEGIN" => Some(Kind::Begin),
            "RESULTS" => Some(Kind::Results),
            "ERROR" => Some(Kind::Error),
            _ => None,
        }
    }
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// With this exit code.
    Code(i32),
    /// Its one worker threw, which elm-test, running it in its own
    /// process, catches as it catches its own errors; the message is
    /// shown.
    Thrown,
}

impl Supervise {
    /// Runs the tests of the program at `program`, printing to `console`;
    /// how it ended.
    pub fn run(self, console: &mut dyn Console, program: &Path) -> io::Result<Ended> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let (send, events) = mpsc::channel();
        let accepting = accept(listener, send.clone());
        let mut workers = Vec::new();
        for index in 0..self.processes {
            // `<program> <index> <port>`, as the driver and `after.js`
            // read their arguments.
            let mut command = Command::new("bun");
            if self.processes == 1 {
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
        let mut state = State::new(self);
        let code = state.supervise(console, &events, &mut workers);
        settle(console, &events, &mut workers);
        state.summarize(console);
        // The listener waits for a connection: this one, which it drops as
        // nothing receives it any more.
        drop(events);
        let _ = TcpStream::connect((Ipv4Addr::LOCALHOST, port));
        let _ = accepting.join();
        Ok(code)
    }
}

struct Worker {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    exited: bool,
}

/// Ends the workers and forwards what they printed and the supervisor has
/// not yet read, which a worker prints before it sends what ends the run.
fn settle(console: &mut dyn Console, events: &Receiver<Event>, workers: &mut [Worker]) {
    for worker in workers.iter_mut() {
        let _ = worker.child.kill();
        let _ = worker.child.wait();
    }
    // A reader ends once nothing holds its pipe open, which a process a
    // worker started still may: it gets a moment only.
    let deadline = Instant::now() + Duration::from_secs(1);
    let reading = |workers: &[Worker]| {
        workers
            .iter()
            .flat_map(|w| &w.readers)
            .any(|r| !r.is_finished())
    };
    while reading(workers) && Instant::now() < deadline {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(10)) {
            print_output(console, event);
        }
    }
    while let Ok(event) = events.try_recv() {
        print_output(console, event);
    }
}

/// Prints `event` if it is what a worker printed.
fn print_output(console: &mut dyn Console, event: Event) {
    match event {
        Event::Out(text) => console.out(&text),
        Event::Err(text) => console.err(&text),
        Event::Connected(..) | Event::Line(..) => {}
    }
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

/// The results due to print, from `BEGIN` on.
#[derive(Clone, Copy, Debug)]
struct Due {
    /// The next one's index.
    next: usize,
    tests: usize,
}

/// The results a run keeps, as its report prints them.
enum Kept {
    /// The console and JSON reports print each result as it falls due:
    /// those not printed yet, by index, and the todos for the summary.
    Streamed {
        due: Option<Due>,
        waiting: HashMap<usize, Sent>,
        todos: Vec<Value>,
    },
    /// `JUnit` lists every result as the run ends, in the order they came.
    All {
        results: HashMap<usize, Json>,
        order: Vec<usize>,
    },
}

/// What the supervisor knows of the run.
struct State {
    supervise: Supervise,
    kept: Kept,
    finished: usize,
    failures: usize,
    started: Instant,
    connections: HashMap<usize, TcpStream>,
    /// The summary the run ended with, printed once the workers have
    /// ended and what they printed is out.
    summary: Option<Summary>,
}

/// A `SUMMARY` message, kept as sent.
struct Summary {
    exit_code: Option<i32>,
    message: Option<Box<RawValue>>,
}

impl State {
    fn new(supervise: Supervise) -> State {
        let kept = match supervise.report {
            Report::JUnit => Kept::All {
                results: HashMap::new(),
                order: Vec::new(),
            },
            Report::Console { .. } | Report::Json => Kept::Streamed {
                due: None,
                waiting: HashMap::new(),
                todos: Vec::new(),
            },
        };
        State {
            supervise,
            kept,
            finished: 0,
            failures: 0,
            started: Instant::now(),
            connections: HashMap::new(),
            summary: None,
        }
    }

    fn supervise(
        &mut self,
        console: &mut dyn Console,
        events: &Receiver<Event>,
        workers: &mut [Worker],
    ) -> Ended {
        loop {
            match events.recv_timeout(Duration::from_millis(20)) {
                Ok(event) => {
                    if let Some(code) = self.event(console, event) {
                        return Ended::Code(code);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ended::Code(1),
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
    ) -> Option<Ended> {
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
                    return Some(Ended::Code(code));
                }
            }
            if !status.success() {
                if self.supervise.processes == 1 {
                    return Some(Ended::Thrown);
                }
                let text = "\n\nThere was an unexpected runtime exception while running tests\n\n";
                console.err(&format!("{}\n", chalk(self.supervise.color, 31, text)));
                return Some(Ended::Code(1));
            }
        }
        workers.iter().all(|w| w.exited).then_some(Ended::Code(1))
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
        let Some(kind) = Kind::parse(&message.kind) else {
            console.err(&format!(
                "Unrecognized message from worker:{}\n",
                message.kind
            ));
            return Some(1);
        };
        let report = self.supervise.report;
        match kind {
            Kind::Finished => {
                self.results(console, message.results);
                self.finished += 1;
                if self.finished == self.supervise.processes || self.supervise.processes == 1 {
                    let todos = match &self.kept {
                        Kept::Streamed { todos, .. } => todos.as_slice(),
                        Kept::All { .. } => &[],
                    };
                    let summary = json!({
                        "type": "SUMMARY",
                        "duration": self.started.elapsed().as_millis() as u64,
                        "failures": self.failures,
                        "todos": todos,
                    });
                    if let Some(stream) = self.connections.get_mut(&id) {
                        // A line, as the worker reads its socket by lines.
                        let _ = stream.write_all(format!("{summary}\n").as_bytes());
                    }
                }
                None
            }
            Kind::Summary => {
                self.summary = Some(Summary {
                    exit_code: message.exit_code,
                    message: message.message.map(RawValue::to_owned),
                });
                Some(message.exit_code.unwrap_or(1))
            }
            Kind::Begin => {
                if let Report::Console { .. } = report {
                    let headline = format!("elm-test {ELM_TEST_VERSION}");
                    let bar = "-".repeat(headline.len());
                    console.out(&format!("\n{headline}\n{bar}\n\n"));
                }
                print(console, report, message.message.map(Sent::new).as_ref());
                if let Kept::Streamed { due, .. } = &mut self.kept {
                    *due = message.test_count.map(|tests| Due { next: 0, tests });
                }
                self.flush(console);
                None
            }
            Kind::Results => {
                self.results(console, message.results);
                None
            }
            Kind::Error => {
                let sent = message.message.map(Sent::new);
                let text = sent
                    .as_ref()
                    .map_or(Cow::Borrowed(""), |s| js_string(&s.value));
                console.err(&format!("Error: {text}\n"));
                Some(1)
            }
        }
    }

    /// Prints the summary the run ended with, if it ended with one, after
    /// the results still due.
    fn summarize(&mut self, console: &mut dyn Console) {
        let Some(summary) = self.summary.take() else {
            return;
        };
        self.flush(console);
        let message = summary.message.as_deref();
        if summary.exit_code == Some(1) {
            let sent = message.map(Sent::new);
            let text = sent
                .as_ref()
                .map_or(Cow::Borrowed("undefined"), |s| js_string(&s.value));
            console.err(&format!("{text}\n"));
        } else if let Kept::All { results, order } = &mut self.kept {
            if let Some(summary) = message {
                console.out(&format!("{}\n", junit(results, order, summary)));
            }
        } else {
            print(
                console,
                self.supervise.report,
                message.map(Sent::new).as_ref(),
            );
        }
    }

    /// `handleResults`: each result counted as the report counts them,
    /// kept, and those due printed.
    fn results(&mut self, console: &mut dyn Console, results: Option<HashMap<String, &RawValue>>) {
        // Index order, as JavaScript lists an object's integer keys.
        let mut results: Vec<(usize, &RawValue)> = (results.unwrap_or_default().into_iter())
            .filter_map(|(index, raw)| Some((index.parse().ok()?, raw)))
            .collect();
        results.sort_by_key(|(index, _)| *index);
        let report = self.supervise.report;
        for (index, raw) in results {
            match &mut self.kept {
                Kept::All { results, order } => {
                    let result = parse(raw);
                    if result.get("failure").is_some() {
                        self.failures += 1;
                    }
                    if results.insert(index, result).is_none() {
                        order.push(index);
                    }
                }
                Kept::Streamed {
                    due,
                    waiting,
                    todos,
                } => {
                    let result = Sent::new(raw);
                    match result.status {
                        Some(Status::Fail) => self.failures += 1,
                        Some(Status::Todo) if report == Report::Json => todos.push(json!({
                            "labels": result.value["labels"],
                            "todo": result.value["failures"][0],
                        })),
                        Some(Status::Todo) => todos.push(result.value.clone()),
                        Some(Status::Pass) | None => {}
                    }
                    // One printed already is never printed again.
                    if due.is_none_or(|due| index >= due.next) {
                        waiting.insert(index, result);
                    }
                }
            }
        }
        self.flush(console);
    }

    /// `flushResults`: the results due, in order, while there are. `JUnit`
    /// prints none until the run ends.
    fn flush(&mut self, console: &mut dyn Console) {
        let Kept::Streamed {
            due: Some(due),
            waiting,
            ..
        } = &mut self.kept
        else {
            return;
        };
        while due.next < due.tests
            && let Some(result) = waiting.remove(&due.next)
        {
            print(console, self.supervise.report, Some(&result));
            due.next += 1;
        }
    }
}

/// The `JUnit` report: the summary's XML with every result, taken in the
/// order they came.
fn junit(results: &mut HashMap<usize, Json>, order: &[usize], summary: &RawValue) -> String {
    let results = (order.iter())
        .filter_map(|index| results.remove(index))
        .collect();
    junit::report(parse(summary), results)
}

/// `raw` with its objects' fields in order, or null if it is no JSON.
fn parse(raw: &RawValue) -> Json {
    serde_json::from_str(raw.get()).unwrap_or(Json::Null)
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
            let field = match (result.stage, result.status) {
                (Some(Stage::Begin), _) => "output",
                (Some(Stage::Complete), Some(Status::Pass)) => "distributionReport",
                (Some(Stage::Complete), Some(Status::Fail)) => "failure",
                (Some(Stage::Summary), _) => "summary",
                (Some(Stage::Complete), Some(Status::Todo) | None) | (None, _) => return,
            };
            let Some(text) = result.value.get(field) else {
                return;
            };
            console.out(&format!("{}\n", windows_safe(&js_string(text))));
        }
    }
}

/// JavaScript's `String(value)`.
fn js_string(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(other.to_string()),
    }
}

/// `text` in the foreground color `code` when `color`, as `chalk` writes
/// it.
pub(crate) fn chalk(color: bool, code: u8, text: &str) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[39m")
    } else {
        text.to_owned()
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
pub(crate) fn random_seed() -> u64 {
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
pub(crate) fn default_workers() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}

#[cfg(test)]
mod tests {
    use nomi_engine::console::Captured;

    use super::*;

    /// Results print in index order however they come, each once, and are
    /// let go once printed; `JUnit` keeps every result for its report.
    #[test]
    fn results_print_in_order_once() {
        let lines = [
            r#"{"type":"RESULTS","results":{"1":{"status":"pass"}}}"#,
            r#"{"type":"BEGIN","testCount":3}"#,
            r#"{"type":"RESULTS","results":{"2":{"status":"fail"},"0":{"status":"pass"}}}"#,
            r#"{"type":"RESULTS","results":{"0":{"status":"todo"}}}"#,
        ];
        let run = |report| {
            let mut state = State::new(Supervise {
                report,
                processes: 2,
                color: false,
            });
            let mut console = Captured::default();
            for line in lines {
                assert_eq!(state.response(&mut console, 0, line), None, "{line}");
            }
            (state, console)
        };
        let (state, console) = run(Report::Json);
        assert_eq!(
            console.out,
            "undefined\n{\"status\":\"pass\"}\n{\"status\":\"pass\"}\n{\"status\":\"fail\"}\n"
        );
        let Kept::Streamed { waiting, todos, .. } = &state.kept else {
            panic!("JSON results are streamed")
        };
        assert!(waiting.is_empty());
        assert_eq!((state.failures, todos.len()), (1, 1));
        let (state, console) = run(Report::JUnit);
        assert_eq!(console.out, "");
        let Kept::All { results, order } = &state.kept else {
            panic!("JUnit keeps every result")
        };
        assert_eq!(*order, [1, 0, 2]);
        assert_eq!(
            results[&0].get("status"),
            Some(&Json::String("todo".into()))
        );
    }

    /// What a worker's pipe holds, read only once the test opens the
    /// gate.
    struct Late(Option<&'static [u8]>, Receiver<()>);

    impl Read for Late {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(text) = self.0.take() else {
                return Ok(0);
            };
            let _ = self.1.recv();
            buf[..text.len()].copy_from_slice(text);
            Ok(text.len())
        }
    }

    /// What a worker printed before the run ended prints, before the
    /// summary, however late the supervisor reads it.
    #[test]
    fn what_a_worker_printed_prints_before_the_summary_however_late_it_is_read() {
        nomi_testkit::require_bun();
        let (send, events) = mpsc::channel();
        let child = Command::new("bun")
            .args(["-e", "setInterval(() => {}, 1000)"])
            .spawn()
            .unwrap();
        let (open, gate) = mpsc::channel();
        let late = Late(Some(b"leftover tick\n"), gate);
        let readers = forward(Some(late), send, Event::Out);
        let mut workers = [Worker {
            child,
            readers: readers.into_iter().collect(),
            exited: false,
        }];
        let mut state = State::new(Supervise {
            report: Report::Json,
            processes: 1,
            color: false,
        });
        let mut console = Captured::default();
        let summary = r#"{"type":"SUMMARY","exitCode":0,"message":"done"}"#;
        assert_eq!(state.response(&mut console, 0, summary), Some(0));
        // Read only after the run ended.
        open.send(()).unwrap();
        settle(&mut console, &events, &mut workers);
        state.summarize(&mut console);
        assert_eq!(console.out, "leftover tick\n\"done\"\n");
    }
}
