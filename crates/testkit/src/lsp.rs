//! Language-server sessions: one driver plays the client for
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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, unbounded};
use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::Url;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::process::wait_within;

/// A case: a project and the session run in it.
#[derive(Debug, Clone)]
pub struct Case {
    /// The case's directory's name.
    pub name: String,
    /// The project, with the session's `session.json`.
    pub dir: PathBuf,
    pub session: Session,
}

/// What a session does, from `session.json`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// What the case is, for a case taken from elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Settings over [`default_settings`], as `initializationOptions`
    /// and the `elmLS` section of `workspace/configuration`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub settings: Map<String, Value>,
    /// Files opened, in order, before any request.
    pub open: Vec<String>,
    /// The steps after the files are open, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<Step>,
}

/// One request: its method, the file it's about, and where.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub method: Method,
    /// The file, by path in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Where in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<At>,
    /// More parameters, merged over the ones the driver makes.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
    /// Whether to resolve each item of the answer, in order, recording
    /// the resolved items as an answer of their own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolve: bool,
    /// Files written before the step, by path in the project, with their
    /// text: a file the editor just made.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub create: BTreeMap<String, String>,
}

impl Step {
    /// `method` on `file`, with nothing more.
    pub fn on(method: Method, file: String) -> Step {
        Step {
            method,
            file: Some(file),
            at: None,
            params: Map::new(),
            resolve: false,
            create: BTreeMap::new(),
        }
    }
}

/// An LSP method a step names: the ones the driver makes its own
/// parameters for, answers itself or tidies, or any other.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Method {
    DidOpen,
    DidChange,
    DidSave,
    DidClose,
    CodeAction,
    CodeLens,
    Completion,
    SelectionRange,
    References,
    Rename,
    WillRenameFiles,
    FindTests,
    CodeLensResolve,
    CompletionResolve,
    CodeActionResolve,
    /// Any other notification: a method with `/did` in its name.
    Notification(String),
    /// Any other request.
    Request(String),
}

impl From<String> for Method {
    fn from(method: String) -> Method {
        match method.as_str() {
            "textDocument/didOpen" => Method::DidOpen,
            "textDocument/didChange" => Method::DidChange,
            "textDocument/didSave" => Method::DidSave,
            "textDocument/didClose" => Method::DidClose,
            "textDocument/codeAction" => Method::CodeAction,
            "textDocument/codeLens" => Method::CodeLens,
            "textDocument/completion" => Method::Completion,
            "textDocument/selectionRange" => Method::SelectionRange,
            "textDocument/references" => Method::References,
            "textDocument/rename" => Method::Rename,
            "workspace/willRenameFiles" => Method::WillRenameFiles,
            "elm/findTests" => Method::FindTests,
            "codeLens/resolve" => Method::CodeLensResolve,
            "completionItem/resolve" => Method::CompletionResolve,
            "codeAction/resolve" => Method::CodeActionResolve,
            _ if method.contains("/did") => Method::Notification(method),
            _ => Method::Request(method),
        }
    }
}

impl Method {
    /// The method's name in the protocol.
    pub fn as_str(&self) -> &str {
        match self {
            Method::DidOpen => "textDocument/didOpen",
            Method::DidChange => "textDocument/didChange",
            Method::DidSave => "textDocument/didSave",
            Method::DidClose => "textDocument/didClose",
            Method::CodeAction => "textDocument/codeAction",
            Method::CodeLens => "textDocument/codeLens",
            Method::Completion => "textDocument/completion",
            Method::SelectionRange => "textDocument/selectionRange",
            Method::References => "textDocument/references",
            Method::Rename => "textDocument/rename",
            Method::WillRenameFiles => "workspace/willRenameFiles",
            Method::FindTests => "elm/findTests",
            Method::CodeLensResolve => "codeLens/resolve",
            Method::CompletionResolve => "completionItem/resolve",
            Method::CodeActionResolve => "codeAction/resolve",
            Method::Notification(method) | Method::Request(method) => method,
        }
    }

    /// Whether the client sends the method as a notification.
    fn is_notification(&self) -> bool {
        match self {
            Method::DidOpen
            | Method::DidChange
            | Method::DidSave
            | Method::DidClose
            | Method::Notification(_) => true,
            Method::CodeAction
            | Method::CodeLens
            | Method::Completion
            | Method::SelectionRange
            | Method::References
            | Method::Rename
            | Method::WillRenameFiles
            | Method::FindTests
            | Method::CodeLensResolve
            | Method::CompletionResolve
            | Method::CodeActionResolve
            | Method::Request(_) => false,
        }
    }

    /// The request that resolves an item of the method's answer, if any.
    fn resolve(&self) -> Option<Method> {
        match self {
            Method::CodeLens => Some(Method::CodeLensResolve),
            Method::Completion => Some(Method::CompletionResolve),
            Method::CodeAction => Some(Method::CodeActionResolve),
            Method::DidOpen
            | Method::DidChange
            | Method::DidSave
            | Method::DidClose
            | Method::SelectionRange
            | Method::References
            | Method::Rename
            | Method::WillRenameFiles
            | Method::FindTests
            | Method::CodeLensResolve
            | Method::CompletionResolve
            | Method::CodeActionResolve
            | Method::Notification(_)
            | Method::Request(_) => None,
        }
    }
}

impl Serialize for Method {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A position or range in the step's file: by marker index, or exactly.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(untagged)]
pub enum At {
    Position(usize),
    Range([usize; 2]),
    Exact(Position),
    Span { start: Position, end: Position },
}

/// A zero-based line and UTF-16 column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Position {
    pub line: u32,
    /// The UTF-16 column.
    pub character: u32,
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

/// How long the path of a case's project directory is, in bytes: elm
/// reflows some reports around the project's path, so both servers must
/// see paths of one length for their reports to compare.
const ROOT_LENGTH: usize = 160;

/// A new directory below `base` for the project of `case`, its path
/// padded to [`ROOT_LENGTH`].
pub fn project_dir(base: &Path, case: &str) -> io::Result<PathBuf> {
    crate::padded_dir(&base.join(case), ROOT_LENGTH)
}

/// Copies a case's project to `to`, without its session, and with
/// [`DEFAULT_ELM_JSON`] if it has no `elm.json`.
pub fn copy_project(case: &Case, to: &Path) -> io::Result<()> {
    crate::copy_tree(&case.dir, to, |path| path != Path::new("session.json"))?;
    let manifest = to.join("elm.json");
    if !manifest.exists() {
        std::fs::write(manifest, DEFAULT_ELM_JSON)?;
    }
    Ok(())
}

/// The `elm.json` of a case that has none: an application of the
/// fixture packages.
pub const DEFAULT_ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": [
        "src"
    ],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {
            "elm/browser": "1.0.2",
            "elm/core": "1.0.5",
            "elm/html": "1.0.1",
            "elm/http": "2.0.0",
            "elm/json": "1.1.4",
            "elm/url": "1.0.0"
        },
        "indirect": {
            "elm/bytes": "1.0.8",
            "elm/file": "1.0.5",
            "elm/time": "1.0.0",
            "elm/virtual-dom": "1.0.5"
        }
    },
    "test-dependencies": {
        "direct": {},
        "indirect": {}
    }
}
"#;

/// The settings every session starts from: no diagnostics as you type,
/// so elm-language-server's own approximate type checker stays out of
/// the transcript (DECISIONS.md, 2026-10-12). On open and save it still
/// runs `elm make`, and its lint rules too, whatever
/// `disableElmLSDiagnostics` says.
pub fn default_settings() -> Map<String, Value> {
    let Value::Object(settings) = json!({
        "onlyUpdateDiagnosticsOnSave": true,
        "disableElmLSDiagnostics": true,
        "elmReviewDiagnostics": "off",
        "skipInstallPackageConfirmation": false,
        "trace": { "server": "off" },
    }) else {
        unreachable!("an object literal")
    };
    settings
}

/// The `--^` markers of `text`, in order: for each, the zero-based line
/// above it and the UTF-16 column of its `^`. A marker line may hold
/// several.
fn markers(text: &str) -> Vec<Position> {
    let mut markers = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let trimmed = content.trim_start();
        if line == 0 || !trimmed.starts_with("--^") {
            continue;
        }
        for (at, _) in content.match_indices("--^") {
            let column = content[..at + 2].encode_utf16().count();
            markers.push(Position {
                line: line as u32 - 1,
                character: column as u32,
            });
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

/// The client's end of a connection to a language server, which sends
/// each request under an id of its own.
pub struct Client {
    pub connection: Connection,
    sent: i32,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client over `connection`, no request sent yet.
    pub fn new(connection: Connection) -> Client {
        Client {
            connection,
            sent: 0,
        }
    }

    /// Sends a request under a new id, which it gives back.
    pub fn send_request(&mut self, method: &str, params: Value) -> RequestId {
        self.sent += 1;
        let id = RequestId::from(self.sent);
        self.send(Message::Request(Request::new(
            id.clone(),
            method.to_owned(),
            params,
        )));
        id
    }

    /// The answer to the request `id`, waited for at most `within`; what
    /// else the server sends meanwhile goes to `other`.
    pub fn response(
        &self,
        id: &RequestId,
        within: Duration,
        mut other: impl FnMut(Message),
    ) -> Response {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.connection.receiver.recv_timeout(left) {
                Ok(Message::Response(response)) if response.id == *id => return response,
                Ok(message) => other(message),
                Err(RecvTimeoutError::Timeout) => panic!("no answer to request {id}"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the server left before answering request {id}")
                }
            }
        }
    }

    /// Sends the notification `method` with `params`.
    pub fn notify(&self, method: &str, params: Value) {
        self.send(Message::Notification(Notification::new(
            method.to_owned(),
            params,
        )));
    }

    /// Answers a request the server sent.
    pub fn respond(&self, response: Response) {
        self.send(Message::Response(response));
    }

    fn send(&self, message: Message) {
        self.connection
            .sender
            .send(message)
            .expect("the server is listening");
    }
}

/// The URI of the directory `dir`, without the last `/`, as servers
/// name a project.
pub fn dir_uri(dir: &Path) -> String {
    let uri = Url::from_directory_path(dir).expect("an absolute directory");
    uri.as_str().trim_end_matches('/').to_owned()
}

/// What to write for a directory a transcript names by each of `names`,
/// as a URI and as a path: `to`. A resolved name holds the one behind a
/// link, so it comes first.
pub fn dir_replacements(names: &[&Path], to: &str) -> Vec<(String, String)> {
    (names.iter())
        .flat_map(|name| [dir_uri(name), name.display().to_string()])
        .map(|name| (name, to.to_owned()))
        .collect()
}

/// Waits for a child server to exit after `exit`, killing it after a
/// while.
pub fn reap(mut child: Child) {
    let _ = wait_within(
        &mut child,
        Duration::from_secs(5),
        Duration::from_millis(20),
    );
}

/// How long to wait.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Silence that counts as the server having settled.
    pub quiet: Duration,
    /// The longest any one answer may take.
    pub answer: Duration,
    /// Whether the server publishes diagnostics for every file it opens,
    /// so settling waits for those too, however slow the machine.
    pub publishes: bool,
}

/// The client side of a session.
pub struct Driver {
    client: Client,
    root: PathBuf,
    timing: Timing,
    workspace: Workspace,
    /// The files open, by path in the project.
    open: BTreeMap<String, OpenFile>,
}

/// What the client keeps of what the server sends unasked, and answers
/// its requests from.
struct Workspace {
    root_uri: String,
    settings: Map<String, Value>,
    /// The latest diagnostics published, by URI.
    published: BTreeMap<String, Value>,
    /// Edits the server applied through `workspace/applyEdit`, since the
    /// last request.
    applied: Vec<Value>,
    /// Opened files whose diagnostics haven't come yet, by URI.
    awaiting: BTreeSet<String>,
}

/// A file open in the editor.
#[derive(Debug)]
struct OpenFile {
    version: i32,
    /// What the editor writes to disk on a save.
    text: String,
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
    /// and waits for it to settle.
    pub fn start(connection: Connection, root: &Path, session: &Session, timing: Timing) -> Driver {
        let mut settings = default_settings();
        settings.extend(session.settings.clone());
        let mut driver = Driver {
            client: Client::new(connection),
            root: root.to_owned(),
            timing,
            workspace: Workspace {
                root_uri: dir_uri(root),
                settings,
                published: BTreeMap::new(),
                applied: Vec::new(),
                awaiting: BTreeSet::new(),
            },
            open: BTreeMap::new(),
        };
        let root_uri = &driver.workspace.root_uri;
        let params = json!({
            "processId": null,
            "rootUri": root_uri,
            "workspaceFolders": [{ "uri": root_uri, "name": "case" }],
            "initializationOptions": driver.workspace.settings,
            "capabilities": client_capabilities(),
        });
        let initialized = driver.request("initialize", params);
        assert!(initialized.is_ok(), "initialize: {initialized:?}");
        driver.client.notify("initialized", json!({}));
        driver.settle();
        driver
    }

    /// Runs `session`, then shuts the server down, and gives back the
    /// transcript.
    pub fn run(mut self, session: &Session) -> Value {
        for file in &session.open {
            self.open_file(file);
        }
        self.settle();
        let diagnostics = self.diagnostics(session.open.iter().cloned());
        let mut requests = Vec::new();
        for step in &session.requests {
            for (file, text) in &step.create {
                let path = self.root.join(file);
                std::fs::create_dir_all(path.parent().expect("a file in the project")).unwrap();
                std::fs::write(&path, text).unwrap();
            }
            // The editor's own: what it publishes then, once it's quiet.
            if let Some(entry) = self.document_event(step) {
                requests.push(entry);
                continue;
            }
            let params = self.params(step);
            // A notification: what the server applies in answer, once it's
            // quiet again.
            if step.method.is_notification() {
                self.client.notify(step.method.as_str(), params);
                self.settle();
                requests.push(json!({
                    "method": step.method,
                    "applied": std::mem::take(&mut self.workspace.applied),
                }));
                continue;
            }
            let answer = self.request(step.method.as_str(), params);
            let resolved = match (&answer, step.resolve) {
                (Ok(Value::Array(items)), true) => Some(self.resolve(&step.method, items.clone())),
                _ => None,
            };
            let mut entry = Map::new();
            entry.insert("method".into(), json!(step.method));
            match answer {
                Ok(result) => entry.insert("result".into(), result),
                Err(error) => entry.insert("error".into(), error),
            };
            if !self.workspace.applied.is_empty() {
                entry.insert(
                    "applied".into(),
                    Value::Array(std::mem::take(&mut self.workspace.applied)),
                );
            }
            requests.push(Value::Object(entry));
            requests.extend(resolved);
        }
        let shutdown = self.request("shutdown", Value::Null);
        assert!(shutdown.is_ok(), "shutdown: {shutdown:?}");
        self.client.notify("exit", Value::Null);
        json!({ "diagnostics": diagnostics, "requests": requests })
    }

    /// Each of `items` resolved, as one entry: the resolve request of
    /// the method that answered them, and the resolved items in order.
    fn resolve(&mut self, method: &Method, items: Vec<Value>) -> Value {
        let resolve = (method.resolve())
            .unwrap_or_else(|| panic!("{} has no resolve request", method.as_str()));
        let results: Vec<Value> = items
            .into_iter()
            .map(|item| {
                self.request(resolve.as_str(), item)
                    .unwrap_or_else(|error| json!({ "error": error }))
            })
            .collect();
        json!({ "method": resolve, "result": results })
    }

    fn open_file(&mut self, file: &str) {
        let text =
            std::fs::read_to_string(self.root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        if self.timing.publishes {
            self.workspace.awaiting.insert(self.uri(file));
        }
        self.open.insert(
            file.to_owned(),
            OpenFile {
                version: 1,
                text: text.clone(),
            },
        );
        self.client.notify(
            Method::DidOpen.as_str(),
            json!({ "textDocument": {
                "uri": self.uri(file), "languageId": "elm", "version": 1, "text": text,
            } }),
        );
    }

    /// A step that opens, changes, saves or closes a document, as an
    /// editor sends them: its entry, with the diagnostics then published
    /// for the open files on an open or save, and for the file on a close.
    /// A change's edits are the step's `contentChanges`.
    fn document_event(&mut self, step: &Step) -> Option<Value> {
        let file = step.file.as_ref()?;
        let method = step.method.as_str();
        let reported: Vec<String> = match step.method {
            Method::DidOpen => {
                self.open_file(file);
                self.settle();
                self.open.keys().cloned().collect()
            }
            Method::DidChange => {
                let uri = self.uri(file);
                let open = (self.open.get_mut(file)).unwrap_or_else(|| panic!("{file} isn't open"));
                open.version += 1;
                let changes = step
                    .params
                    .get("contentChanges")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                for change in changes.as_array().into_iter().flatten() {
                    apply_change(&mut open.text, change);
                }
                let params = json!({
                    "textDocument": { "uri": uri, "version": open.version },
                    "contentChanges": changes,
                });
                self.client.notify(method, params);
                self.settle();
                Vec::new()
            }
            Method::DidSave => {
                // An editor writes the file, then says so.
                if let Some(open) = self.open.get(file) {
                    std::fs::write(self.root.join(file), &open.text).unwrap();
                }
                if self.timing.publishes {
                    let open: Vec<String> = self.open.keys().map(|f| self.uri(f)).collect();
                    self.workspace.awaiting.extend(open);
                }
                (self.client).notify(method, json!({ "textDocument": { "uri": self.uri(file) } }));
                self.settle();
                self.open.keys().cloned().collect()
            }
            Method::DidClose => {
                self.open.remove(file);
                (self.client).notify(method, json!({ "textDocument": { "uri": self.uri(file) } }));
                self.settle();
                vec![file.clone()]
            }
            Method::CodeAction
            | Method::CodeLens
            | Method::Completion
            | Method::SelectionRange
            | Method::References
            | Method::Rename
            | Method::WillRenameFiles
            | Method::FindTests
            | Method::CodeLensResolve
            | Method::CompletionResolve
            | Method::CodeActionResolve
            | Method::Notification(_)
            | Method::Request(_) => return None,
        };
        let mut entry = json!({ "method": step.method, "file": file });
        if !reported.is_empty() {
            entry["diagnostics"] = Value::Object(self.diagnostics(reported));
        }
        Some(entry)
    }

    /// The diagnostics last published for each of `files`, `[]` for one
    /// with none.
    fn diagnostics(&self, files: impl IntoIterator<Item = String>) -> Map<String, Value> {
        (files.into_iter())
            .map(|file| {
                let published = self.workspace.published.get(&self.uri(&file)).cloned();
                (file, published.unwrap_or_else(|| json!([])))
            })
            .collect()
    }

    fn uri(&self, file: &str) -> String {
        format!("{}/{file}", self.workspace.root_uri)
    }

    fn params(&self, step: &Step) -> Value {
        let mut params = Map::new();
        if let Some(file) = &step.file {
            params.insert("textDocument".into(), json!({ "uri": self.uri(file) }));
            if let Some(at) = step.at {
                let text = std::fs::read_to_string(self.root.join(file)).unwrap();
                let markers = markers(&text);
                let marker = |i: usize| {
                    *markers
                        .get(i)
                        .unwrap_or_else(|| panic!("{file} has no marker {i}"))
                };
                let (start, end) = match at {
                    At::Position(i) => (marker(i), None),
                    At::Range([start, end]) => (marker(start), Some(marker(end))),
                    At::Exact(position) => (position, None),
                    At::Span { start, end } => (start, Some(end)),
                };
                let range = json!({ "start": start, "end": end.unwrap_or(start) });
                match step.method {
                    Method::CodeAction => params.insert("range".into(), range),
                    Method::SelectionRange => params.insert("positions".into(), json!([start])),
                    Method::DidOpen
                    | Method::DidChange
                    | Method::DidSave
                    | Method::DidClose
                    | Method::CodeLens
                    | Method::Completion
                    | Method::References
                    | Method::Rename
                    | Method::WillRenameFiles
                    | Method::FindTests
                    | Method::CodeLensResolve
                    | Method::CompletionResolve
                    | Method::CodeActionResolve
                    | Method::Notification(_)
                    | Method::Request(_) => match end {
                        Some(_) => params.insert("range".into(), range),
                        None => params.insert("position".into(), json!(start)),
                    },
                };
            }
            if step.method == Method::CodeAction {
                let diagnostics = self.workspace.published.get(&self.uri(file)).cloned();
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
        // `<project>` in the step's own stands for the project's URI.
        let project = [("<project>".to_owned(), self.workspace.root_uri.clone())];
        params.extend((step.params.iter()).map(|(key, value)| {
            (
                replace_all(key.clone(), &project),
                normalize(value.clone(), &project),
            )
        }));
        Value::Object(params)
    }

    /// Sends a request and waits for its answer, handling everything
    /// else that comes meanwhile: the result, or the error as JSON.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.client.send_request(method, params);
        let response = (self.client).response(&id, self.timing.answer, |message| {
            self.workspace.handle(&self.client, message);
        });
        match response.error {
            Some(error) => Err(json!({ "code": error.code, "message": error.message })),
            None => Ok(response.result.unwrap_or(Value::Null)),
        }
    }

    /// Handles messages until the server says nothing for a while, and
    /// has published for every file awaited.
    fn settle(&mut self) {
        let deadline = Instant::now() + self.timing.answer;
        loop {
            match self
                .client
                .connection
                .receiver
                .recv_timeout(self.timing.quiet)
            {
                Ok(message) => self.workspace.handle(&self.client, message),
                Err(RecvTimeoutError::Timeout) if self.workspace.awaiting.is_empty() => return,
                Err(RecvTimeoutError::Timeout) if Instant::now() > deadline => {
                    panic!("no diagnostics for {:?}", self.workspace.awaiting)
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("the server left"),
            }
        }
    }
}

impl Workspace {
    fn handle(&mut self, client: &Client, message: Message) {
        match message {
            Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => {
                // elm-language-server percent-encodes them, `@` as `%40`.
                let uri = percent_decoded(n.params["uri"].as_str().unwrap_or_default());
                self.awaiting.remove(&uri);
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
                client.respond(Response::new_ok(request.id, result));
            }
            _ => {}
        }
    }
}

/// `text` with an LSP content change made: a range in UTF-16 columns
/// replaced, or all of it where the change has no range.
fn apply_change(text: &mut String, change: &Value) {
    let new = change["text"].as_str().unwrap_or_default();
    let Some(range) = change.get("range") else {
        *text = new.to_owned();
        return;
    };
    let offset = |position: &Value| {
        let line = position["line"].as_u64().unwrap_or(0) as usize;
        let column = position["character"].as_u64().unwrap_or(0) as usize;
        let start = text
            .split_inclusive('\n')
            .take(line)
            .map(str::len)
            .sum::<usize>();
        let mut units = 0;
        let mut at = start;
        for c in text[start..].chars() {
            if units >= column || c == '\n' {
                break;
            }
            units += c.len_utf16();
            at += c.len_utf8();
        }
        at
    };
    let (start, end) = (offset(&range["start"]), offset(&range["end"]));
    text.replace_range(start..end.max(start), new);
}

/// What the driver's client supports: what VS Code offers that the
/// servers read, with edits given eagerly rather than resolved later.
pub fn client_capabilities() -> Value {
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
            "fileOperations": { "didCreate": true, "willRename": true, "willDelete": true },
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

/// A transcript as stored: `normalize`d, references in order, and each
/// completion item's documentation, the bulk of a completion list, kept
/// as a digest.
pub fn tidy(transcript: Value, replacements: &[(String, String)]) -> Value {
    let mut transcript = normalize(transcript, replacements);
    let requests = transcript["requests"].as_array_mut();
    for request in requests.into_iter().flatten() {
        let Some(method) = request["method"].as_str().map(str::to_owned) else {
            continue;
        };
        match Method::from(method) {
            // Servers find references in no particular order across files.
            Method::References => sort_by_json(request["result"].as_array_mut()),
            // The test modules come in the order the disk lists them.
            Method::FindTests => {
                let suites = request.pointer_mut("/result/suites");
                if let Some(suites) = suites.and_then(Value::as_array_mut) {
                    suites.sort_by_key(|s| s["file"].as_str().unwrap_or_default().to_owned());
                }
            }
            Method::CodeLensResolve => {
                for lens in request["result"].as_array_mut().into_iter().flatten() {
                    sort_by_json(
                        lens.pointer_mut("/command/arguments/0/references")
                            .and_then(Value::as_array_mut),
                    );
                }
            }
            Method::Rename | Method::WillRenameFiles => tidy_rename(&mut request["result"]),
            Method::Completion => tidy_completion(&mut request["result"]),
            Method::DidOpen
            | Method::DidChange
            | Method::DidSave
            | Method::DidClose
            | Method::CodeAction
            | Method::CodeLens
            | Method::SelectionRange
            | Method::CompletionResolve
            | Method::CodeActionResolve
            | Method::Notification(_)
            | Method::Request(_) => {}
        }
    }
    transcript
}

/// `values`, where there are some, sorted by their JSON: a list whose
/// order a server doesn't fix.
fn sort_by_json(values: Option<&mut Vec<Value>>) {
    if let Some(values) = values {
        values.sort_by_cached_key(|value| {
            serde_json::to_string(value).expect("JSON values serialize")
        });
    }
}

/// A rename's edits in order: servers don't order them.
fn tidy_rename(result: &mut Value) {
    if let Some(changes) = result.get_mut("changes").and_then(Value::as_object_mut) {
        for edits in changes.values_mut() {
            sort_by_json(edits.as_array_mut());
        }
    }
    let documents = result.get_mut("documentChanges");
    if let Some(documents) = documents.and_then(Value::as_array_mut) {
        for document in documents.iter_mut() {
            sort_by_json(document.get_mut("edits").and_then(Value::as_array_mut));
        }
        sort_by_json(Some(documents));
    }
}

/// A completion list with each item's documentation, the bulk of the
/// list, kept as a digest.
fn tidy_completion(result: &mut Value) {
    let items = match result {
        Value::Array(items) => Some(items),
        Value::Object(list) => list.get_mut("items").and_then(Value::as_array_mut),
        _ => None,
    };
    for item in items.into_iter().flatten() {
        // Tidied once already, it stays as it is.
        if let Some(documentation) = item.get_mut("documentation")
            && documentation.get("sha256").is_none()
        {
            let text = serde_json::to_string(documentation).expect("JSON values serialize");
            let digest = crate::sha256_hex(text.as_bytes());
            *documentation = json!({ "sha256": &digest[..16] });
        }
    }
}

/// `uri` with its percent escapes decoded.
pub fn percent_decoded(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| uri.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escaped {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A transcript as written to a file: indented to [`TEXT_DEPTH`], each
/// value deeper on one line, so a completion item or a location takes a
/// line.
pub fn to_text(value: &Value) -> String {
    fn write(value: &Value, depth: usize, out: &mut String) {
        let indent = |out: &mut String, depth: usize| {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', 2 * depth));
        };
        match value {
            Value::Array(items) if depth < TEXT_DEPTH && !items.is_empty() => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    indent(out, depth + 1);
                    write(item, depth + 1, out);
                }
                indent(out, depth);
                out.push(']');
            }
            Value::Object(map) if depth < TEXT_DEPTH && !map.is_empty() => {
                out.push('{');
                for (i, (key, item)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    indent(out, depth + 1);
                    out.push_str(&serde_json::to_string(key).expect("JSON values serialize"));
                    out.push_str(": ");
                    write(item, depth + 1, out);
                }
                indent(out, depth);
                out.push('}');
            }
            value => out.push_str(&serde_json::to_string(value).expect("JSON values serialize")),
        }
    }
    let mut out = String::new();
    write(value, 0, &mut out);
    out.push('\n');
    out
}

/// How deep [`to_text`] indents.
const TEXT_DEPTH: usize = 5;

/// `value` with every string's occurrences of each `from` replaced by
/// its `to`, in order, so longer prefixes go first; keys too.
fn normalize(value: Value, replacements: &[(String, String)]) -> Value {
    match value {
        Value::String(text) => Value::String(replace_all(text, replacements)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| normalize(v, replacements))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (replace_all(k, replacements), normalize(v, replacements)))
                .collect(),
        ),
        other => other,
    }
}

/// `text` with the occurrences of each `from` replaced by its `to`, in
/// order.
fn replace_all(mut text: String, replacements: &[(String, String)]) -> String {
    for (from, to) in replacements {
        if text.contains(from.as_str()) {
            text = text.replace(from.as_str(), to);
        }
    }
    text
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
                serde_json::to_string(e).expect("JSON values serialize"),
                serde_json::to_string(a).expect("JSON values serialize")
            ))
        }
    };
    let none = Vec::new();
    let expected_requests = expected["requests"].as_array().unwrap_or(&none);
    let actual_requests = actual["requests"].as_array().unwrap_or(&none);
    // Answers past the recorded ones fail the session's own outcome, as
    // the ratchet fails only cases it lists.
    let extra = actual_requests
        .len()
        .saturating_sub(expected_requests.len());
    let session = compare(&expected["diagnostics"], &actual["diagnostics"]).and_then(|()| {
        if extra == 0 {
            Ok(())
        } else {
            Err(format!("{extra} answers more than were recorded"))
        }
    });
    let mut outcomes = vec![(format!("{case}/diagnostics"), session)];
    let (expected, actual) = (expected_requests, actual_requests);
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
    fn an_answer_more_than_recorded_fails_the_session() {
        let expected = serde_json::json!({ "diagnostics": {}, "requests": [{ "method": "a" }] });
        let actual = serde_json::json!({
            "diagnostics": {},
            "requests": [{ "method": "a" }, { "method": "b" }],
        });
        let found = outcomes("c", &expected, &actual);
        assert_eq!(found.len(), 2);
        assert!(found[0].1.as_ref().unwrap_err().contains("1 answers more"));
        assert_eq!(found[1].1, Ok(()));
        assert!(
            outcomes("c", &expected, &expected)
                .iter()
                .all(|(_, r)| r.is_ok())
        );
    }

    #[test]
    fn markers_point_at_the_line_above() {
        let text =
            "module Main exposing (..)\n\nmain =\n    text \"hi\"\n  --^  --^\n    --^ again\n";
        let at = |line, character| Position { line, character };
        assert_eq!(markers(text), [at(3, 4), at(3, 9), at(4, 6)]);
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
