//! `nomi lsp`: the Language Server Protocol over the engine (architecture
//! §9). This crate holds the protocol, positions and the order of work;
//! what the answers are is `nomi_ide`'s. The editor's text lies over the
//! disk in the engine's VFS, and diagnostics are worked out on the pool
//! against a snapshot and published only while it is current.

mod diagnostics;
mod edits;
mod indexes;
mod position;
mod requests;
mod settings;

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crossbeam_channel::Sender;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeConfiguration, DidChangeTextDocument, DidCloseTextDocument, DidCreateFiles,
    DidOpenTextDocument, DidSaveTextDocument, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, CodeActionResolveRequest, CodeLensRequest, CodeLensResolve, Completion,
    DocumentHighlightRequest, DocumentSymbolRequest, ExecuteCommand, FoldingRangeRequest,
    Formatting, GotoDefinition, GotoTypeDefinition, HoverRequest, LinkedEditingRange,
    PrepareRenameRequest, References, Rename, Request as _, SelectionRangeRequest, WillDeleteFiles,
    WillRenameFiles, WorkspaceConfiguration, WorkspaceSymbolRequest,
};
use lsp_types::{
    ClientCapabilities, CodeActionOptions, CodeActionProviderCapability, CodeLensOptions,
    CompletionOptions, ConfigurationItem, ConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, ExecuteCommandOptions,
    FileOperationFilter, FileOperationPattern, FileOperationPatternKind,
    FileOperationRegistrationOptions, FoldingRangeProviderCapability, HoverProviderCapability,
    InitializeParams, InitializeResult, LinkedEditingRangeServerCapabilities, MessageActionItem,
    OneOf, PositionEncodingKind, PublishDiagnosticsParams, RenameOptions,
    SelectionRangeProviderCapability, ServerCapabilities, ServerInfo, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, TypeDefinitionProviderCapability, Url,
    WorkspaceFileOperationsServerCapabilities, WorkspaceServerCapabilities,
};
use nomi_base::{TextSize, lock};
use nomi_engine::artifacts::Artifacts;
use nomi_engine::index::{self, Index, Memo, PackageModules};
use nomi_engine::paths::Cache;
use nomi_engine::{Snapshot, Store, Vfs};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use diagnostics::{Cause, Diagnostics, Job, Shared};
use edits::{Answer, Choice, INSTALL_PACKAGE, Install};
use indexes::Indexes;
use position::Text;
use settings::Settings;

/// Why serving stopped before the client said to exit.
#[derive(Debug)]
pub enum Error {
    Protocol(lsp_server::ProtocolError),
    /// The `initialize` request's parameters aren't LSP's.
    Initialize(serde_json::Error),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Protocol(e) => write!(f, "{e}"),
            Error::Initialize(e) => write!(f, "bad initialize parameters: {e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<lsp_server::ProtocolError> for Error {
    fn from(e: lsp_server::ProtocolError) -> Error {
        Error::Protocol(e)
    }
}

/// Serves the client on standard input and output until it exits.
pub fn stdio() -> Result<(), Error> {
    let (connection, io) = Connection::stdio();
    run(&connection)?;
    drop(connection);
    io.join().map_err(Error::Io)
}

/// Serves the client at the other end of `connection` until it exits:
/// the handshake, then one message at a time. Packages come from the
/// store in nomi's cache.
pub fn run(connection: &Connection) -> Result<(), Error> {
    // Without a cache directory, packages are nowhere to be found.
    let cache = Cache::locate().ok();
    let store = Store::new(cache.as_ref().map_or_else(PathBuf::new, Cache::packages));
    let artifacts = cache.as_ref().map(Artifacts::new);
    // An editor may be all that builds: it trims the cache as a build
    // does, once it starts, without holding up the handshake.
    if let Some(artifacts) = artifacts.clone() {
        std::thread::spawn(move || artifacts.trim());
    }
    serve_with(connection, store, Memo::new(), artifacts)
}

/// As [`run`], with packages from `store`, and `memo` holding those
/// already built; each check from scratch.
pub fn serve(connection: &Connection, store: Store, memo: Memo) -> Result<(), Error> {
    serve_with(connection, store, memo, None)
}

/// As [`serve`], checks reusing what `artifacts` holds.
fn serve_with(
    connection: &Connection,
    store: Store,
    memo: Memo,
    artifacts: Option<Artifacts>,
) -> Result<(), Error> {
    // Checks and indexes read the same files again and again.
    nomi_engine::graph::keep_reads();
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params).map_err(Error::Initialize)?;
    let offered = (params.capabilities.general.as_ref())
        .and_then(|general| general.position_encodings.as_deref());
    let (encoding, unit) = position::negotiate(offered);
    let result = InitializeResult {
        capabilities: capabilities(encoding),
        server_info: Some(ServerInfo {
            name: "nomi".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    };
    let result = serde_json::to_value(result).expect("the handshake's answer serializes");
    connection.initialize_finish(id, result)?;
    let settings = (params.initialization_options.as_ref())
        .and_then(Settings::read)
        .unwrap_or_default();
    let shared = Arc::new(Shared {
        store,
        indexes: Indexes::default(),
        artifacts,
        memo: Mutex::new(memo),
        unit,
        sender: connection.sender.clone(),
        publishing: Mutex::new(()),
        settings: Mutex::new(settings),
        latest: Mutex::new(HashMap::new()),
    });
    let mut server = Server {
        diagnostics: Diagnostics::start(shared.clone()),
        shared,
        vfs: Vfs::new(),
        documents: HashMap::new(),
        client: ClientSupport::of(&params.capabilities),
        settings_request: None,
        sent: 0,
        packages: OnceLock::new(),
        installs: HashMap::new(),
    };
    server.serve(connection)
}

/// What the server offers, as elm-language-server does, its positions in
/// `encoding`.
fn capabilities(encoding: PositionEncodingKind) -> ServerCapabilities {
    // The file events elm-language-server listens to: Elm files, and for
    // a rename, folders too.
    let filter = |glob: &str, matches| FileOperationFilter {
        scheme: Some("file".to_owned()),
        pattern: FileOperationPattern {
            glob: glob.to_owned(),
            matches: Some(matches),
            options: None,
        },
    };
    let elm_files = || filter("**/*.elm", FileOperationPatternKind::File);
    let folders = filter("**/", FileOperationPatternKind::Folder);
    let filters = |filters| Some(FileOperationRegistrationOptions { filters });
    ServerCapabilities {
        position_encoding: Some(encoding),
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                ..TextDocumentSyncOptions::default()
            },
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
        folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
        linked_editing_range_provider: Some(LinkedEditingRangeServerCapabilities::Simple(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
        })),
        references_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            resolve_provider: Some(true),
            ..CodeActionOptions::default()
        })),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![INSTALL_PACKAGE.to_owned()],
            work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
        }),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(true),
        }),
        document_highlight_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_owned()]),
            ..CompletionOptions::default()
        }),
        workspace: Some(WorkspaceServerCapabilities {
            workspace_folders: None,
            file_operations: Some(WorkspaceFileOperationsServerCapabilities {
                did_create: filters(vec![elm_files()]),
                will_rename: filters(vec![elm_files(), folders]),
                will_delete: filters(vec![elm_files()]),
                ..WorkspaceFileOperationsServerCapabilities::default()
            }),
        }),
        ..ServerCapabilities::default()
    }
}

/// What the client does that the server's answers depend on.
#[derive(Clone, Copy, Debug)]
struct ClientSupport {
    /// Whether it shows completions' label details.
    label_details: bool,
    /// Whether it resolves a code action's edit later.
    resolves_edits: bool,
    /// Whether it answers `workspace/configuration`.
    pulls_settings: bool,
}

impl ClientSupport {
    fn of(capabilities: &ClientCapabilities) -> ClientSupport {
        let text_document = capabilities.text_document.as_ref();
        ClientSupport {
            label_details: (text_document.and_then(|t| t.completion.as_ref()))
                .and_then(|c| c.completion_item.as_ref())
                .and_then(|i| i.label_details_support)
                .unwrap_or(false),
            resolves_edits: (text_document.and_then(|t| t.code_action.as_ref()))
                .and_then(|c| c.resolve_support.as_ref())
                .is_some_and(|r| r.properties.iter().any(|p| p == "edit")),
            pulls_settings: (capabilities.workspace.as_ref())
                .and_then(|w| w.configuration)
                .unwrap_or(false),
        }
    }
}

struct Server {
    /// What the diagnostics worker shares.
    shared: Arc<Shared>,
    vfs: Vfs,
    /// The documents the client has open, with their text.
    documents: HashMap<Url, Document>,
    diagnostics: Diagnostics,
    client: ClientSupport,
    /// The id of the request for the client's settings, while it's out.
    settings_request: Option<RequestId>,
    /// How many requests the server has sent the client, each under an
    /// id of its own.
    sent: i32,
    /// The packages downloaded and the modules they expose, read once.
    packages: OnceLock<PackageModules>,
    /// Installs waiting for the user's answer, by the id of the question.
    installs: HashMap<RequestId, Install>,
}

struct Document {
    path: PathBuf,
    text: String,
}

/// A position in an open document: the document, its text, and the
/// offset of the position in it.
struct At<'a> {
    document: &'a Document,
    text: Text<'a>,
    offset: TextSize,
}

/// Why a request has no answer: the protocol's code and a message.
struct Failure {
    code: ErrorCode,
    message: String,
}

impl Failure {
    /// The request's parameters aren't what it takes.
    fn invalid(message: impl fmt::Display) -> Failure {
        Failure {
            code: ErrorCode::InvalidParams,
            message: message.to_string(),
        }
    }

    /// Answering failed, as a file that can't be read makes it.
    fn internal(error: impl fmt::Display) -> Failure {
        Failure {
            code: ErrorCode::InternalError,
            message: error.to_string(),
        }
    }

    /// The request is well formed but has no answer.
    fn failed(message: impl fmt::Display) -> Failure {
        Failure {
            code: ErrorCode::RequestFailed,
            message: message.to_string(),
        }
    }
}

impl Server {
    /// Answers the client's messages one at a time until it exits.
    fn serve(&mut self, connection: &Connection) -> Result<(), Error> {
        for message in &connection.receiver {
            match message {
                Message::Request(request) => {
                    if connection.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    // A bug answers its request with an error rather than
                    // end the session.
                    let id = request.id.clone();
                    let method = request.method.clone();
                    let response = catch_unwind(AssertUnwindSafe(|| self.request(request)))
                        .unwrap_or_else(|_| {
                            let message =
                                format!("nomi failed answering {method}; this is a bug in nomi");
                            Response::new_err(id, ErrorCode::InternalError as i32, message)
                        });
                    send(&self.shared.sender, Message::Response(response));
                }
                Message::Notification(notification) => {
                    let _ = catch_unwind(AssertUnwindSafe(|| self.notification(notification)));
                }
                Message::Response(response) => self.response(&response),
            }
        }
        Ok(())
    }

    fn request(&mut self, Request { id, method, params }: Request) -> Response {
        let result = match method.as_str() {
            Formatting::METHOD => {
                answer::<Formatting>(params, |p| self.formatting(&p.text_document.uri))
            }
            DocumentSymbolRequest::METHOD => {
                answer::<DocumentSymbolRequest>(params, |p| self.symbols(&p.text_document.uri))
            }
            GotoDefinition::METHOD => answer::<GotoDefinition>(params, |p| {
                self.goto(&p.text_document_position_params, nomi_ide::definition)
            }),
            GotoTypeDefinition::METHOD => answer::<GotoTypeDefinition>(params, |p| {
                self.goto(&p.text_document_position_params, nomi_ide::type_definition)
            }),
            FoldingRangeRequest::METHOD => {
                answer::<FoldingRangeRequest>(params, |p| self.folding(&p.text_document.uri))
            }
            SelectionRangeRequest::METHOD => answer::<SelectionRangeRequest>(params, |p| {
                self.selection(&p.text_document.uri, &p.positions)
            }),
            LinkedEditingRange::METHOD => answer::<LinkedEditingRange>(params, |p| {
                self.linked(&p.text_document_position_params)
            }),
            WorkspaceSymbolRequest::METHOD => answer::<WorkspaceSymbolRequest>(params, |p| {
                Ok(Some(self.workspace_symbols(&p.query)))
            }),
            PrepareRenameRequest::METHOD => {
                answer::<PrepareRenameRequest>(params, |p| self.prepare_rename(&p))
            }
            Rename::METHOD => answer::<Rename>(params, |p| {
                self.rename(&p.text_document_position, &p.new_name)
            }),
            References::METHOD => answer::<References>(params, |p| {
                self.references(&p.text_document_position, p.context.include_declaration)
            }),
            HoverRequest::METHOD => {
                answer::<HoverRequest>(params, |p| self.hover(&p.text_document_position_params))
            }
            Completion::METHOD => {
                answer::<Completion>(params, |p| self.completion(&p.text_document_position))
            }
            DocumentHighlightRequest::METHOD => answer::<DocumentHighlightRequest>(params, |p| {
                self.highlights(&p.text_document_position_params)
            }),
            CodeLensRequest::METHOD => {
                answer::<CodeLensRequest>(params, |p| self.code_lenses(&p.text_document.uri))
            }
            CodeLensResolve::METHOD => {
                answer::<CodeLensResolve>(params, |lens| self.resolve_lens(lens))
            }
            // Its diagnostics carry a `uri` the protocol's type drops.
            CodeActionRequest::METHOD => parse(params).and_then(|p| self.code_actions(&p)),
            CodeActionResolveRequest::METHOD => self.resolve_action(params),
            ExecuteCommand::METHOD => answer::<ExecuteCommand>(params, |p| self.execute(&p)),
            WillRenameFiles::METHOD => {
                answer::<WillRenameFiles>(params, |p| Ok(Some(self.rename_files(&p))))
            }
            WillDeleteFiles::METHOD => Ok(Value::Null),
            "elm/provideFileContents" => self.file_contents(&params).map(Value::String),
            "workspace/textDocumentContent" => {
                (self.file_contents(&params)).map(|text| json!({ "text": text }))
            }
            "elm/findTests" => self.find_tests(&params),
            "elm/expose" => self.expose(&params, nomi_ide::Exposure::Expose),
            "elm/unexpose" => self.expose(&params, nomi_ide::Exposure::Unexpose),
            "elm/getMoveDestinations" => Ok(self.move_destinations(&params)),
            "elm/move" => Ok(self.move_to(&params)),
            "elm/getDiagnostics" => {
                // A check now, published as any other is.
                self.check_now();
                Ok(Value::Null)
            }
            method => Err(Failure {
                code: ErrorCode::MethodNotFound,
                message: format!("nomi doesn't answer {method} yet"),
            }),
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err(failure) => Response::new_err(id, failure.code as i32, failure.message),
        }
    }

    fn notification(&mut self, Notification { method, params }: Notification) {
        match method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(params) {
                    self.open(&p.text_document.uri, p.text_document.text);
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(params) {
                    self.change(p);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
                    self.close(&p.text_document.uri);
                }
            }
            DidSaveTextDocument::METHOD => self.check_now(),
            Initialized::METHOD => self.pull_settings(),
            DidCreateFiles::METHOD => {
                if let Ok(p) = serde_json::from_value(params) {
                    self.created_files(&p);
                }
            }
            DidChangeConfiguration::METHOD => {
                if let Some(settings) = params.get("settings").and_then(Settings::read) {
                    *lock(&self.shared.settings) = settings;
                }
                self.pull_settings();
            }
            _ => {}
        }
    }

    /// Asks the client for its `elmLS` settings, where it answers such
    /// requests.
    fn pull_settings(&mut self) {
        if self.client.pulls_settings {
            let params = ConfigurationParams {
                items: vec![ConfigurationItem {
                    scope_uri: None,
                    section: Some("elmLS".to_owned()),
                }],
            };
            self.settings_request = Some(self.ask::<WorkspaceConfiguration>(params));
        }
    }

    fn response(&mut self, response: &Response) {
        let result = response.result.as_ref();
        if let Some(install) = self.installs.remove(&response.id) {
            let agreed = (result.and_then(|r| Option::<MessageActionItem>::deserialize(r).ok()))
                .flatten()
                .is_some_and(|item| item.title == Choice::Yes.title());
            if agreed {
                let installed = self.install(&install.root, &install.package, Answer::Yes);
                let printed = installed.printed.replacen(&install.printed, "", 1);
                self.show(lsp_types::MessageType::INFO, printed.trim());
            }
        } else if self.settings_request.as_ref() == Some(&response.id) {
            self.settings_request = None;
            if let Some(settings) = result.and_then(|r| r.get(0)).and_then(Settings::read) {
                *lock(&self.shared.settings) = settings;
            }
        }
    }

    fn open(&mut self, uri: &Url, text: String) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        self.documents.insert(uri.clone(), Document { path, text });
        let document = &self.documents[uri];
        self.revise(Cause::Saved, |vfs| {
            let _ = vfs.set_overlay(&document.path, document.text.as_str());
        });
    }

    fn change(&mut self, params: DidChangeTextDocumentParams) {
        let uri = &params.text_document.uri;
        let unit = self.shared.unit;
        let Some(document) = self.documents.get_mut(uri) else {
            return;
        };
        for change in params.content_changes {
            match change.range {
                None => document.text = change.text,
                Some(range) => {
                    let range = Text::new(&document.text, unit).text_range(range);
                    document
                        .text
                        .replace_range(std::ops::Range::<usize>::from(range), &change.text);
                }
            }
        }
        let document = &self.documents[uri];
        self.revise(Cause::Edited(document.path.clone()), |vfs| {
            let _ = vfs.set_overlay(&document.path, document.text.as_str());
        });
    }

    fn close(&mut self, uri: &Url) {
        if let Some(document) = self.documents.remove(uri) {
            self.revise(Cause::Closed, |vfs| {
                let _ = vfs.remove_overlay(&document.path);
            });
            let params = PublishDiagnosticsParams::new(uri.clone(), Vec::new(), None);
            send(
                &self.shared.sender,
                notification::<PublishDiagnostics>(params),
            );
        }
    }

    /// Has the files checked as they are now: a file saved, or the
    /// client asking.
    fn check_now(&self) {
        self.revise(Cause::Saved, |vfs| {
            vfs.disk_changed();
        });
    }

    /// Makes a revision, `change` laying the editor's text over the disk
    /// or taking it off, and has it checked for `cause`.
    fn revise(&self, cause: Cause, change: impl FnOnce(&Vfs)) {
        let snapshot = {
            let _held = lock(&self.shared.publishing);
            change(&self.vfs);
            self.vfs.snapshot()
        };
        let open = (self.documents.iter())
            .map(|(uri, document)| (uri.clone(), document.path.clone()))
            .collect();
        self.diagnostics.check(Job {
            snapshot,
            open,
            cause,
        });
    }

    fn document(&self, uri: &Url) -> Result<&Document, Failure> {
        (self.documents.get(uri)).ok_or_else(|| Failure::invalid(format!("{uri} isn't open")))
    }

    fn at(&self, position: &TextDocumentPositionParams) -> Result<At<'_>, Failure> {
        let document = self.document(&position.text_document.uri)?;
        let text = Text::new(&document.text, self.shared.unit);
        let offset = text.offset(position.position);
        Ok(At {
            document,
            text,
            offset,
        })
    }

    /// The index of the project that owns `path`, made anew only when the
    /// editor's text changed since the last.
    fn index(&self, snapshot: &Snapshot, path: &Path) -> Option<Arc<Index>> {
        let mut memo = lock(&self.shared.memo);
        (self.shared.indexes).get(snapshot, &self.shared.store, &mut memo, path)
    }

    /// Sends the client a request under an id no other has had, which its
    /// answer comes back with.
    fn ask<R: lsp_types::request::Request>(&mut self, params: R::Params) -> RequestId {
        self.sent += 1;
        let id = RequestId::from(self.sent);
        let request = Request::new(id.clone(), R::METHOD.to_owned(), params);
        send(&self.shared.sender, Message::Request(request));
        id
    }

    fn show(&self, kind: lsp_types::MessageType, message: &str) {
        let params = lsp_types::ShowMessageParams {
            typ: kind,
            message: message.to_owned(),
        };
        send(
            &self.shared.sender,
            notification::<lsp_types::notification::ShowMessage>(params),
        );
    }
}

/// The answer to a request of type `R`: `answer` of its parameters.
fn answer<R: lsp_types::request::Request>(
    params: Value,
    answer: impl FnOnce(R::Params) -> Result<R::Result, Failure>,
) -> Result<Value, Failure> {
    let result = answer(parse(params)?)?;
    Ok(serde_json::to_value(result).expect("the protocol's types serialize"))
}

fn parse<P: DeserializeOwned>(params: Value) -> Result<P, Failure> {
    serde_json::from_value(params).map_err(Failure::invalid)
}

/// The file a `file:` URI names.
fn file_path(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

/// The directory of the project that owns `path`.
fn project_root(snapshot: &Snapshot, path: &Path) -> Option<PathBuf> {
    index::manifest(snapshot, path)?
        .parent()
        .map(Path::to_path_buf)
}

/// The project that owns `path` as elm-language-server's commands name
/// it: its directory's URI without the last `/`.
fn command_root(snapshot: &Snapshot, path: &Path) -> Option<String> {
    let root = Url::from_file_path(project_root(snapshot, path)?).ok()?;
    Some(root.as_str().trim_end_matches('/').to_owned())
}

/// Runs `publish` if `snapshot` is still the latest revision, holding
/// `publishing` so that no newer one is made meanwhile. Whether it ran.
fn publish_if_current(publishing: &Mutex<()>, snapshot: &Snapshot, publish: impl FnOnce()) -> bool {
    let _held = lock(publishing);
    if snapshot.is_stale() {
        return false;
    }
    publish();
    true
}

fn notification<N: lsp_types::notification::Notification>(params: N::Params) -> Message {
    Message::Notification(Notification::new(N::METHOD.to_owned(), params))
}

/// Sends `message`; once the client is gone there is no one to tell.
fn send(sender: &Sender<Message>, message: Message) {
    let _ = sender.send(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_snapshot_publishes_nothing() {
        let vfs = Vfs::new();
        let publishing = Mutex::new(());
        let snapshot = vfs.snapshot();
        assert!(publish_if_current(&publishing, &snapshot, || {}));
        vfs.disk_changed();
        let mut published = false;
        assert!(!publish_if_current(&publishing, &snapshot, || published = true));
        assert!(!published);
    }
}
