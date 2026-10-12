//! Language-server sessions (plan 9.1): one driver plays the client for
//! both elm-language-server, recorded as the oracle, and `nomi lsp`,
//! replayed against it, so both see the same session.
//!
//! A case is a project directory with a `session.json`: the settings,
//! the files to open, and the requests to make. Requests point into a
//! file at markers written as elm-language-server's own tests write them:
//! a comment line `--^` stands for the position of its `^` on the line
//! above. The driver answers what servers ask of clients, waits for the
//! server to go quiet after opening files, and writes the transcript as
//! JSON: the diagnostics published for the opened files, then each
//! request's answer. Paths are replaced by names that are the same for
//! every run and server.

use std::collections::BTreeMap;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, unbounded};
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// A case: a project and the session run in it.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    pub dir: PathBuf,
    pub session: Session,
}

/// What a session does, from `session.json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Settings over [`default_settings`], as `initializationOptions`
    /// and the `elmLS` section of `workspace/configuration`.
    #[serde(default)]
    pub settings: Map<String, Value>,
    /// Files opened, in order, before any request.
    pub open: Vec<String>,
    #[serde(default)]
    pub requests: Vec<Step>,
}

/// One request: its method, the file it's about, and where.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub method: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub at: Option<At>,
    /// More parameters, merged over the ones the driver makes.
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// A position or range, by marker index in the step's file.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum At {
    Position(usize),
    Range([usize; 2]),
}

/// The cases under `dir`, each a directory holding `session.json`, by
/// name.
pub fn cases(dir: &Path) -> Vec<Case> {
    let mut cases = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let dir = entry.unwrap().path();
        let session = dir.join("session.json");
        if !session.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&session).unwrap();
        let session =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", session.display()));
        cases.push(Case {
            name: dir.file_name().unwrap().to_string_lossy().into_owned(),
            dir,
            session,
        });
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// Copies a case's project to `to`, without its session.
pub fn copy_project(case: &Case, to: &Path) -> io::Result<()> {
    fn copy(from: &Path, to: &Path, top: bool) -> io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let name = entry.file_name();
            if top && name == "session.json" {
                continue;
            }
            if entry.file_type()?.is_dir() {
                copy(&entry.path(), &to.join(&name), false)?;
            } else {
                std::fs::copy(entry.path(), to.join(&name))?;
            }
        }
        Ok(())
    }
    copy(&case.dir, to, true)
}

/// The settings every session starts from: no diagnostics as you type,
/// so elm-language-server's own approximate type checker stays out of
/// the transcript (DECISIONS.md, 2026-10-12). On open and save it still
/// runs `elm make`, and its lint rules too, whatever
/// `disableElmLSDiagnostics` says.
pub fn default_settings() -> Map<String, Value> {
    let settings = json!({
        "onlyUpdateDiagnosticsOnSave": true,
        "disableElmLSDiagnostics": true,
        "elmReviewDiagnostics": "off",
        "skipInstallPackageConfirmation": false,
        "trace": { "server": "off" },
    });
    match settings {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

/// The `--^` markers of `text`, in order: for each, the zero-based line
/// above it and the UTF-16 column of its `^`. A marker line may hold
/// several.
pub fn markers(text: &str) -> Vec<(u32, u32)> {
    let mut markers = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let trimmed = content.trim_start();
        if line == 0 || !trimmed.starts_with("--^") {
            continue;
        }
        for (at, _) in content.match_indices("--^") {
            let column = content[..at + 2].encode_utf16().count();
            markers.push((line as u32 - 1, column as u32));
        }
    }
    markers
}

/// A connection to a server running as a child process, over its
/// standard streams.
pub fn child_connection(stdin: ChildStdin, stdout: ChildStdout) -> Connection {
    let (to_server, from_client) = unbounded::<Message>();
    let (to_client, from_server) = unbounded::<Message>();
    thread::spawn(move || {
        let mut stdin = stdin;
        for message in from_client {
            if message.write(&mut stdin).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        while let Ok(Some(message)) = Message::read(&mut stdout) {
            if to_client.send(message).is_err() {
                break;
            }
        }
    });
    Connection {
        sender: to_server,
        receiver: from_server,
    }
}

/// Waits for a child server to exit after `exit`, killing it after a
/// while.
pub fn reap(mut child: Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// How long to wait.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Silence that counts as the server having settled.
    pub quiet: Duration,
    /// The longest any one answer may take.
    pub answer: Duration,
}

/// The client side of a session.
pub struct Driver {
    connection: Connection,
    root: PathBuf,
    root_uri: String,
    settings: Map<String, Value>,
    timing: Timing,
    next_id: i32,
    /// The latest diagnostics published, by URI.
    published: BTreeMap<String, Value>,
    /// Edits the server applied through `workspace/applyEdit`, since the
    /// last request.
    applied: Vec<Value>,
}

impl std::fmt::Debug for Driver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Driver")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Driver {
    /// Initializes the server at `connection` for the project at `root`,
    /// whose URI is `root_uri`, and waits for it to settle.
    pub fn start(
        connection: Connection,
        root: &Path,
        root_uri: &str,
        session: &Session,
        timing: Timing,
    ) -> Driver {
        let mut settings = default_settings();
        settings.extend(session.settings.clone());
        let mut driver = Driver {
            connection,
            root: root.to_owned(),
            root_uri: root_uri.trim_end_matches('/').to_owned(),
            settings,
            timing,
            next_id: 0,
            published: BTreeMap::new(),
            applied: Vec::new(),
        };
        let params = json!({
            "processId": null,
            "rootUri": driver.root_uri,
            "workspaceFolders": [{ "uri": driver.root_uri, "name": "case" }],
            "initializationOptions": driver.settings,
            "capabilities": client_capabilities(),
        });
        let initialized = driver.request("initialize", params);
        assert!(initialized.is_ok(), "initialize: {initialized:?}");
        driver.notify("initialized", json!({}));
        driver.settle();
        driver
    }

    /// Runs `session`, then shuts the server down, and gives back the
    /// transcript.
    pub fn run(mut self, session: &Session) -> Value {
        for file in &session.open {
            let text = std::fs::read_to_string(self.root.join(file))
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            self.notify(
                "textDocument/didOpen",
                json!({ "textDocument": {
                    "uri": self.uri(file), "languageId": "elm", "version": 1, "text": text,
                } }),
            );
        }
        self.settle();
        let mut diagnostics = Map::new();
        for file in &session.open {
            let published = self.published.get(&self.uri(file)).cloned();
            diagnostics.insert(file.clone(), published.unwrap_or_else(|| json!([])));
        }
        let mut requests = Vec::new();
        for step in &session.requests {
            let params = self.params(step);
            let answer = self.request(&step.method, params);
            let mut entry = Map::new();
            entry.insert("method".into(), json!(step.method));
            match answer {
                Ok(result) => entry.insert("result".into(), result),
                Err(error) => entry.insert("error".into(), error),
            };
            if !self.applied.is_empty() {
                entry.insert(
                    "applied".into(),
                    Value::Array(std::mem::take(&mut self.applied)),
                );
            }
            requests.push(Value::Object(entry));
        }
        let shutdown = self.request("shutdown", Value::Null);
        assert!(shutdown.is_ok(), "shutdown: {shutdown:?}");
        self.notify("exit", Value::Null);
        json!({ "diagnostics": diagnostics, "requests": requests })
    }

    fn uri(&self, file: &str) -> String {
        format!("{}/{file}", self.root_uri)
    }

    fn params(&self, step: &Step) -> Value {
        let mut params = Map::new();
        if let Some(file) = &step.file {
            params.insert("textDocument".into(), json!({ "uri": self.uri(file) }));
            if let Some(at) = step.at {
                let text = std::fs::read_to_string(self.root.join(file)).unwrap();
                let markers = markers(&text);
                let position = |i: usize| {
                    let (line, character) = *markers
                        .get(i)
                        .unwrap_or_else(|| panic!("{file} has no marker {i}"));
                    json!({ "line": line, "character": character })
                };
                match at {
                    At::Position(i) if step.method == "textDocument/codeAction" => {
                        let range = json!({ "start": position(i), "end": position(i) });
                        params.insert("range".into(), range);
                    }
                    At::Position(i) if step.method == "textDocument/selectionRange" => {
                        params.insert("positions".into(), json!([position(i)]));
                    }
                    At::Position(i) => {
                        params.insert("position".into(), position(i));
                    }
                    At::Range([start, end]) => {
                        let range = json!({ "start": position(start), "end": position(end) });
                        params.insert("range".into(), range);
                    }
                }
            }
            if step.method == "textDocument/codeAction" {
                let diagnostics = self.published.get(&self.uri(file)).cloned();
                let range = params.get("range").cloned().unwrap_or(Value::Null);
                let within: Vec<Value> = (diagnostics.unwrap_or_else(|| json!([])))
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|d| overlaps(&d["range"], &range))
                    .cloned()
                    .collect();
                params.insert("context".into(), json!({ "diagnostics": within }));
            }
        }
        params.extend(step.params.clone());
        Value::Object(params)
    }

    fn notify(&self, method: &str, params: Value) {
        let notification = Notification::new(method.to_owned(), params);
        self.send(Message::Notification(notification));
    }

    fn send(&self, message: Message) {
        self.connection
            .sender
            .send(message)
            .expect("the server is listening");
    }

    /// Sends a request and waits for its answer, handling everything
    /// else that comes meanwhile: the result, or the error as JSON.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.send(Message::Request(Request::new(
            id.clone(),
            method.to_owned(),
            params,
        )));
        let deadline = Instant::now() + self.timing.answer;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = match self.connection.receiver.recv_timeout(left) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => panic!("no answer to {method}"),
                Err(RecvTimeoutError::Disconnected) => panic!("the server left during {method}"),
            };
            match message {
                Message::Response(response) if response.id == id => {
                    return match response.error {
                        Some(error) => Err(json!({ "code": error.code, "message": error.message })),
                        None => Ok(response.result.unwrap_or(Value::Null)),
                    };
                }
                message => self.handle(message),
            }
        }
    }

    /// Handles messages until the server says nothing for a while.
    fn settle(&mut self) {
        loop {
            match self.connection.receiver.recv_timeout(self.timing.quiet) {
                Ok(message) => self.handle(message),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => panic!("the server left"),
            }
        }
    }

    fn handle(&mut self, message: Message) {
        match message {
            Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => {
                let uri = n.params["uri"].as_str().unwrap_or_default().to_owned();
                self.published.insert(uri, n.params["diagnostics"].clone());
            }
            Message::Request(request) => {
                let result = match request.method.as_str() {
                    "workspace/configuration" => {
                        let items = request.params["items"].as_array().map_or(0, Vec::len);
                        Value::Array(vec![Value::Object(self.settings.clone()); items])
                    }
                    "workspace/applyEdit" => {
                        self.applied.push(request.params["edit"].clone());
                        json!({ "applied": true })
                    }
                    "workspace/workspaceFolders" => {
                        json!([{ "uri": self.root_uri, "name": "case" }])
                    }
                    _ => Value::Null,
                };
                self.send(Message::Response(Response::new_ok(request.id, result)));
            }
            _ => {}
        }
    }
}

/// What the driver's client supports: what VS Code offers that the
/// servers read, with edits given eagerly rather than resolved later.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "workspace": {
            "applyEdit": true,
            "configuration": true,
            "workspaceFolders": true,
            "workspaceEdit": {
                "documentChanges": true,
                "resourceOperations": ["create", "rename", "delete"],
            },
            "symbol": {},
        },
        "textDocument": {
            "synchronization": { "didSave": true },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "completion": {
                "completionItem": { "snippetSupport": true, "labelDetailsSupport": true },
            },
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": { "valueSet": [
                        "quickfix", "refactor", "refactor.extract", "refactor.move",
                    ] },
                },
                "isPreferredSupport": true,
            },
            "rename": { "prepareSupport": true },
            "publishDiagnostics": { "relatedInformation": true, "tagSupport": { "valueSet": [1, 2] } },
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": false },
        },
    })
}

/// Whether two LSP ranges share a position, a missing one sharing
/// every position.
fn overlaps(a: &Value, b: &Value) -> bool {
    let key = |p: &Value| (p["line"].as_u64(), p["character"].as_u64());
    if a.is_null() || b.is_null() {
        return true;
    }
    key(&a["start"]) <= key(&b["end"]) && key(&b["start"]) <= key(&a["end"])
}

/// `value` with every string's occurrences of each `from` replaced by
/// its `to`, in order, so longer prefixes go first.
pub fn normalize(value: Value, replacements: &[(String, String)]) -> Value {
    match value {
        Value::String(text) => {
            let mut text = text;
            for (from, to) in replacements {
                if text.contains(from.as_str()) {
                    text = text.replace(from.as_str(), to);
                }
            }
            Value::String(text)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| normalize(v, replacements))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let k = match normalize(Value::String(k), replacements) {
                        Value::String(k) => k,
                        _ => unreachable!(),
                    };
                    (k, normalize(v, replacements))
                })
                .collect(),
        ),
        other => other,
    }
}

/// The ratchet's outcomes for one case: its diagnostics, then each
/// request, compared with the oracle's transcript.
pub fn outcomes(case: &str, expected: &Value, actual: &Value) -> Vec<(String, Result<(), String>)> {
    let compare = |e: &Value, a: &Value| {
        if e == a {
            Ok(())
        } else {
            Err(format!(
                "expected {}\n   found {}",
                serde_json::to_string(e).unwrap(),
                serde_json::to_string(a).unwrap()
            ))
        }
    };
    let mut outcomes = vec![(
        format!("{case}/diagnostics"),
        compare(&expected["diagnostics"], &actual["diagnostics"]),
    )];
    let none = Vec::new();
    let expected = expected["requests"].as_array().unwrap_or(&none);
    let actual = actual["requests"].as_array().unwrap_or(&none);
    for (i, e) in expected.iter().enumerate() {
        let method = e["method"].as_str().unwrap_or_default();
        let a = actual.get(i).unwrap_or(&Value::Null);
        outcomes.push((format!("{case}/{i:02} {method}"), compare(e, a)));
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_point_at_the_line_above() {
        let text =
            "module Main exposing (..)\n\nmain =\n    text \"hi\"\n  --^  --^\n    --^ again\n";
        assert_eq!(markers(text), [(3, 4), (3, 9), (4, 6)]);
    }

    #[test]
    fn normalizing_replaces_in_keys_and_values() {
        let value = json!({ "file:///tmp/x/src/A.elm": ["file:///tmp/x/src/B.elm", 1] });
        let replaced = normalize(value, &[("file:///tmp/x".into(), "<project>".into())]);
        assert_eq!(
            replaced,
            json!({ "<project>/src/A.elm": ["<project>/src/B.elm", 1] })
        );
    }

    #[test]
    fn ranges_overlap_where_they_share_a_position() {
        let range = |a: u64, b: u64| json!({ "start": { "line": a, "character": 0 }, "end": { "line": b, "character": 0 } });
        assert!(overlaps(&range(1, 3), &range(3, 4)));
        assert!(!overlaps(&range(1, 2), &range(3, 4)));
        assert!(overlaps(&Value::Null, &range(3, 4)));
    }
}
