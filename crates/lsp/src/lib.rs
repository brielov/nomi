//! `nomi lsp`: the Language Server Protocol over the engine (architecture
//! §9). This crate holds the protocol, positions and the order of work;
//! what the answers are is `nomi_ide`'s. The editor's text lies over the
//! disk in the engine's VFS, and diagnostics are worked out on the pool
//! against a snapshot and published only while it is current (CR.2).

mod diagnostics;
mod position;

use std::collections::HashMap;

use lsp_types::{DiagnosticSeverity, DiagnosticTag};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeConfiguration, DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
    DidSaveTextDocument, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentHighlightRequest, DocumentSymbolRequest, Formatting, GotoDefinition,
    GotoTypeDefinition, HoverRequest, PrepareRenameRequest, References, Rename, Request as _,
};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentHighlight, DocumentHighlightParams, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, InitializeParams,
    Location, OneOf, PublishDiagnosticsParams, ReferenceParams, ServerCapabilities, ServerInfo,
    SymbolInformation, SymbolKind, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Url,
};
use nomi_base::{ColumnUnit, TextSize};
use nomi_engine::index::{self, Index, Memo};
use nomi_engine::paths::Cache;
use nomi_engine::{Revision, Snapshot, Store, Vfs};
use serde_json::Value;

use diagnostics::{Diagnostics, Job, Settings, Shared};
use position::Text;

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
    let store = Store::new(Cache::locate().map_or_else(|_| PathBuf::new(), |c| c.packages()));
    serve(connection, store, Memo::new())
}

/// As [`run`], with packages from `store`, and `memo` holding those
/// already built.
pub fn serve(connection: &Connection, store: Store, memo: Memo) -> Result<(), Error> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params).map_err(Error::Initialize)?;
    let offered = params
        .capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_deref());
    let unit = position::negotiate(offered);
    let capabilities = ServerCapabilities {
        position_encoding: Some(position::kind(unit)),
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
        type_definition_provider: Some(lsp_types::TypeDefinitionProviderCapability::Simple(true)),
        folding_range_provider: Some(lsp_types::FoldingRangeProviderCapability::Simple(true)),
        selection_range_provider: Some(lsp_types::SelectionRangeProviderCapability::Simple(true)),
        linked_editing_range_provider: Some(
            lsp_types::LinkedEditingRangeServerCapabilities::Simple(true),
        ),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Right(lsp_types::RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
        })),
        references_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Options(
            lsp_types::CodeActionOptions {
                resolve_provider: Some(true),
                ..lsp_types::CodeActionOptions::default()
            },
        )),
        execute_command_provider: Some(lsp_types::ExecuteCommandOptions {
            commands: vec![INSTALL_PACKAGE.to_owned()],
            work_done_progress_options: lsp_types::WorkDoneProgressOptions::default(),
        }),
        code_lens_provider: Some(lsp_types::CodeLensOptions {
            resolve_provider: Some(true),
        }),
        document_highlight_provider: Some(OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        completion_provider: Some(lsp_types::CompletionOptions {
            trigger_characters: Some(vec![".".to_owned()]),
            ..lsp_types::CompletionOptions::default()
        }),
        ..ServerCapabilities::default()
    };
    let mut capabilities = serde_json::to_value(capabilities).expect("capabilities serialize");
    // The file events elm-language-server listens to: Elm files, and for
    // a rename, folders too.
    let elm_files = serde_json::json!({ "scheme": "file", "pattern": { "glob": "**/*.elm", "matches": "file" } });
    let folders =
        serde_json::json!({ "scheme": "file", "pattern": { "glob": "**/", "matches": "folder" } });
    capabilities["workspace"]["fileOperations"] = serde_json::json!({
        "didCreate": { "filters": [elm_files] },
        "willRename": { "filters": [elm_files, folders] },
        "willDelete": { "filters": [elm_files] },
    });
    let result = serde_json::json!({
        "capabilities": capabilities,
        "serverInfo": ServerInfo {
            name: "nomi".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    });
    connection.initialize_finish(id, result)?;
    let settings = (params.initialization_options.as_ref())
        .and_then(Settings::read)
        .unwrap_or_default();
    let label_details = (params.capabilities.text_document.as_ref())
        .and_then(|t| t.completion.as_ref())
        .and_then(|c| c.completion_item.as_ref())
        .and_then(|i| i.label_details_support)
        .unwrap_or(false);
    let resolves_edits = (params.capabilities.text_document.as_ref())
        .and_then(|t| t.code_action.as_ref())
        .and_then(|c| c.resolve_support.as_ref())
        .is_some_and(|r| r.properties.iter().any(|p| p == "edit"));
    let pulls_settings = (params.capabilities.workspace.as_ref())
        .and_then(|w| w.configuration)
        .unwrap_or(false);
    let settings = Arc::new(Mutex::new(settings));
    let publishing = Arc::new(Mutex::new(()));
    let memo = Arc::new(Mutex::new(memo));
    let latest = Arc::new(Mutex::new(HashMap::new()));
    let diagnostics = Diagnostics::start(Shared {
        store: store.clone(),
        memo: memo.clone(),
        unit,
        sender: connection.sender.clone(),
        publishing: publishing.clone(),
        settings: settings.clone(),
        latest: latest.clone(),
    });
    let mut server = Server {
        unit,
        vfs: Arc::new(Vfs::new()),
        documents: HashMap::new(),
        publishing,
        sender: connection.sender.clone(),
        store,
        memo,
        index: None,
        diagnostics,
        settings,
        pulls_settings,
        settings_request: None,
        label_details,
        latest,
        resolves_edits,
        packages: None,
        installs: HashMap::new(),
    };
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                let response = server.request(request);
                send(&server.sender, Message::Response(response));
            }
            Message::Notification(notification) => server.notification(notification),
            Message::Response(response) => server.response(response),
        }
    }
    Ok(())
}

mod edits;

use edits::{INSTALL_PACKAGE, Install, install_package};

struct Server {
    unit: ColumnUnit,
    vfs: Arc<Vfs>,
    /// The documents the client has open, with their text.
    documents: HashMap<Url, Document>,
    /// Held to make a revision, and to publish what a snapshot found while
    /// it is still the latest, so nothing stale is published after
    /// something newer.
    publishing: Arc<Mutex<()>>,
    sender: Sender<Message>,
    /// Where navigation finds the packages a project uses, never
    /// downloading them.
    store: Store,
    /// The packages' interfaces, made once per server and shared with
    /// diagnostics.
    memo: Arc<Mutex<Memo>>,
    /// The last project index, its revision and its `elm.json`: indexing
    /// a project takes a while, and navigation asks again and again
    /// between edits.
    index: Option<(Revision, PathBuf, Index)>,
    diagnostics: Diagnostics,
    settings: Arc<Mutex<Settings>>,
    /// Whether the client answers `workspace/configuration`.
    pulls_settings: bool,
    /// The id of the request for the client's settings, while it's out.
    settings_request: Option<lsp_server::RequestId>,
    /// Whether the client shows completions' label details.
    label_details: bool,
    /// The problems the diagnostics worker last published, by file.
    latest: Arc<Mutex<HashMap<PathBuf, Vec<nomi_ide::Problem>>>>,
    /// Whether the client resolves a code action's edit later.
    resolves_edits: bool,
    /// The packages downloaded and the modules they expose, read once.
    packages: Option<nomi_ide::PackageModules>,
    /// Installs waiting for the user's answer, by the id of the question.
    installs: HashMap<lsp_server::RequestId, Install>,
}

struct Document {
    path: PathBuf,
    text: String,
}

impl Server {
    fn request(&mut self, request: Request) -> Response {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            Formatting::METHOD => parse(request)
                .and_then(|p: DocumentFormattingParams| self.formatting(&p.text_document.uri)),
            DocumentSymbolRequest::METHOD => parse(request)
                .and_then(|p: DocumentSymbolParams| self.symbols(&p.text_document.uri)),
            GotoDefinition::METHOD => parse(request).and_then(|p: GotoDefinitionParams| {
                self.definition(&p.text_document_position_params)
            }),
            GotoTypeDefinition::METHOD => parse(request).and_then(|p: GotoDefinitionParams| {
                self.type_definition(&p.text_document_position_params)
            }),
            "textDocument/foldingRange" => parse(request)
                .and_then(|p: lsp_types::FoldingRangeParams| self.folding(&p.text_document.uri)),
            "textDocument/selectionRange" => {
                parse(request).and_then(|p: lsp_types::SelectionRangeParams| {
                    self.selection(&p.text_document.uri, &p.positions)
                })
            }
            "textDocument/linkedEditingRange" => {
                parse(request).and_then(|p: lsp_types::LinkedEditingRangeParams| {
                    self.linked(&p.text_document_position_params)
                })
            }
            "workspace/symbol" => parse(request)
                .and_then(|p: lsp_types::WorkspaceSymbolParams| self.workspace_symbols(&p.query)),
            PrepareRenameRequest::METHOD => {
                parse(request).and_then(|p: TextDocumentPositionParams| self.prepare_rename(&p))
            }
            Rename::METHOD => parse(request).and_then(|p: lsp_types::RenameParams| {
                self.rename(&p.text_document_position, &p.new_name)
            }),
            References::METHOD => parse(request).and_then(|p: ReferenceParams| {
                self.references(&p.text_document_position, p.context.include_declaration)
            }),
            HoverRequest::METHOD => parse(request)
                .and_then(|p: lsp_types::HoverParams| self.hover(&p.text_document_position_params)),
            Completion::METHOD => parse(request).and_then(|p: lsp_types::CompletionParams| {
                self.completion(&p.text_document_position)
            }),
            "codeAction/resolve" => parse(request).and_then(|a: Value| self.resolve_action(a)),
            "elm/expose" | "elm/unexpose" => {
                let expose = request.method == "elm/expose";
                parse(request).and_then(|p: Value| self.expose(&p, expose))
            }
            "workspace/willRenameFiles" => {
                parse(request).and_then(|p: lsp_types::RenameFilesParams| self.rename_files(&p))
            }
            "workspace/willDeleteFiles" => Ok(Value::Null),
            "elm/getMoveDestinations" => {
                parse(request).and_then(|p: Value| self.move_destinations(&p))
            }
            "elm/move" => parse(request).and_then(|p: Value| self.move_to(&p)),
            "workspace/executeCommand" => {
                parse(request).and_then(|p: lsp_types::ExecuteCommandParams| self.execute(&p))
            }
            "textDocument/codeAction" => {
                parse(request).and_then(|p: lsp_types::CodeActionParams| self.code_actions(&p))
            }
            "textDocument/codeLens" => parse(request)
                .and_then(|p: lsp_types::CodeLensParams| self.code_lenses(&p.text_document.uri)),
            "codeLens/resolve" => {
                parse(request).and_then(|lens: lsp_types::CodeLens| self.resolve_lens(lens))
            }
            DocumentHighlightRequest::METHOD => {
                parse(request).and_then(|p: DocumentHighlightParams| {
                    self.highlights(&p.text_document_position_params)
                })
            }
            method => Err((
                ErrorCode::MethodNotFound,
                format!("nomi doesn't answer {method} yet"),
            )),
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err((code, message)) => Response::new_err(id, code as i32, message),
        }
    }

    fn notification(&mut self, notification: Notification) {
        match notification.method.as_str() {
            DidOpenTextDocument::METHOD => {
                if let Ok(p) = from::<DidOpenTextDocumentParams>(notification.params) {
                    self.open(p.text_document.uri, p.text_document.text);
                }
            }
            DidChangeTextDocument::METHOD => {
                if let Ok(p) = from::<DidChangeTextDocumentParams>(notification.params) {
                    self.change(p);
                }
            }
            DidCloseTextDocument::METHOD => {
                if let Ok(p) = from::<DidCloseTextDocumentParams>(notification.params) {
                    self.close(&p.text_document.uri);
                }
            }
            DidSaveTextDocument::METHOD => self.revise(true),
            Initialized::METHOD => self.pull_settings(),
            "workspace/didCreateFiles" => {
                if let Ok(p) = from::<lsp_types::CreateFilesParams>(notification.params) {
                    self.created_files(&p);
                }
            }
            DidChangeConfiguration::METHOD => {
                let changed = (notification.params.get("settings")).and_then(Settings::read);
                if let Some(settings) = changed {
                    *self.settings.lock().unwrap() = settings;
                }
                self.pull_settings();
            }
            _ => {}
        }
    }

    /// Asks the client for its `elmLS` settings, where it answers such
    /// requests.
    fn pull_settings(&mut self) {
        if !self.pulls_settings {
            return;
        }
        let id = lsp_server::RequestId::from("nomi/settings".to_owned());
        let params = serde_json::json!({ "items": [{ "section": "elmLS" }] });
        let request = Request::new(id.clone(), "workspace/configuration".to_owned(), params);
        self.settings_request = Some(id);
        send(&self.sender, Message::Request(request));
    }

    fn response(&mut self, response: Response) {
        if let Some(install) = self.installs.remove(&response.id) {
            let agreed = (response.result.as_ref())
                .and_then(|r| r.get("title"))
                .is_some_and(|t| t == "Yes");
            if agreed {
                let (_, printed) = install_package(&install.root, &install.package, Some("y"));
                let printed = printed.replacen(&install.asked, "", 1);
                self.show(lsp_types::MessageType::INFO, printed.trim());
            }
            return;
        }
        if self.settings_request.as_ref() != Some(&response.id) {
            return;
        }
        self.settings_request = None;
        let first = (response.result.as_ref())
            .and_then(|r| r.get(0))
            .and_then(Settings::read);
        if let Some(settings) = first {
            *self.settings.lock().unwrap() = settings;
        }
    }

    fn open(&mut self, uri: Url, text: String) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        self.documents.insert(uri, Document { path, text });
        self.revise(true);
    }

    fn change(&mut self, params: DidChangeTextDocumentParams) {
        let unit = self.unit;
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return;
        };
        for change in params.content_changes {
            match change.range {
                None => document.text = change.text,
                Some(range) => {
                    let text = Text::new(&document.text, unit);
                    let start = usize::from(text.offset(range.start));
                    let end = usize::from(text.offset(range.end)).max(start);
                    document.text.replace_range(start..end, &change.text);
                }
            }
        }
        self.revise(false);
    }

    fn close(&mut self, uri: &Url) {
        if let Some(document) = self.documents.remove(uri) {
            {
                let _held = self.publishing.lock().unwrap();
                let _ = self.vfs.remove_overlay(&document.path);
            }
            self.revise(false);
            let params = PublishDiagnosticsParams::new(uri.clone(), Vec::new(), None);
            send(&self.sender, notification::<PublishDiagnostics>(params));
        }
    }

    /// Lays the open documents over the disk as a new revision, and has
    /// it checked: a file opened or saved for it if `saved`.
    fn revise(&mut self, saved: bool) {
        let snapshot = {
            let _held = self.publishing.lock().unwrap();
            let overlays: Vec<_> = self.documents.values().collect();
            for document in &overlays {
                let _ = self.vfs.set_overlay(&document.path, document.text.as_str());
            }
            self.vfs.snapshot()
        };
        let open = (self.documents.iter())
            .map(|(uri, document)| (uri.clone(), document.path.clone()))
            .collect();
        self.diagnostics.check(Job {
            snapshot,
            open,
            saved,
        });
    }

    fn document(&self, uri: &Url) -> Result<&Document, (ErrorCode, String)> {
        self.documents
            .get(uri)
            .ok_or_else(|| (ErrorCode::InvalidParams, format!("{uri} isn't open")))
    }

    fn formatting(&self, uri: &Url) -> Result<Value, (ErrorCode, String)> {
        let document = self.document(uri)?;
        let edits = nomi_ide::format(&self.vfs.snapshot(), &document.path)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&document.text, self.unit);
        let edits: Option<Vec<TextEdit>> = edits.map(|edits| {
            edits
                .into_iter()
                .map(|e| TextEdit::new(text.range(e.range), e.text))
                .collect()
        });
        Ok(serde_json::to_value(edits).expect("edits serialize"))
    }

    #[allow(deprecated)] // `SymbolInformation::deprecated` must be named.
    fn symbols(&self, uri: &Url) -> Result<Value, (ErrorCode, String)> {
        let document = self.document(uri)?;
        let symbols = nomi_ide::document_symbols(&self.vfs.snapshot(), &document.path)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&document.text, self.unit);
        let symbols: Vec<SymbolInformation> = symbols
            .into_iter()
            .map(|s| SymbolInformation {
                name: s.name,
                kind: symbol_kind(s.kind),
                tags: None,
                deprecated: None,
                location: Location::new(uri.clone(), text.range(s.range)),
                container_name: None,
            })
            .collect();
        let response = DocumentSymbolResponse::Flat(symbols);
        Ok(serde_json::to_value(response).expect("symbols serialize"))
    }
}

impl Server {
    /// The open document a position is in, and the offset of the
    /// position.
    fn at(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<(&Document, TextSize), (ErrorCode, String)> {
        let document = self.document(&position.text_document.uri)?;
        let offset = Text::new(&document.text, self.unit).offset(position.position);
        Ok((document, offset))
    }

    /// The index of the project that owns `path`, made anew only when the
    /// editor's text changed since the last.
    fn index(&mut self, snapshot: &Snapshot, path: &Path) -> Option<&Index> {
        let manifest = index::manifest(snapshot, path)?;
        let revision = snapshot.revision();
        let fresh = (self.index.as_ref()).is_some_and(|(r, m, _)| *r == revision && *m == manifest);
        if !fresh {
            let index = {
                let mut memo = self.memo.lock().unwrap();
                index::index(snapshot, &self.store, path, &mut memo)?
            };
            self.index = Some((revision, manifest, index));
        }
        self.index.as_ref().map(|(_, _, index)| index)
    }

    /// `location` as the protocol names it, its file read through the
    /// snapshot for its positions.
    fn location(&self, snapshot: &Snapshot, location: &nomi_ide::Location) -> Option<Location> {
        let text = snapshot.read(&location.path).ok()?;
        let uri = Url::from_file_path(&location.path).ok()?;
        Some(Location::new(
            uri,
            Text::new(&text, self.unit).range(location.range),
        ))
    }

    fn definition(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .and_then(|index| nomi_ide::definition(&snapshot, index, &path, offset));
        let response = (found.as_ref())
            .and_then(|l| self.location(&snapshot, l))
            .map(GotoDefinitionResponse::Scalar);
        Ok(serde_json::to_value(response).expect("a definition serializes"))
    }

    fn folding(&self, uri: &Url) -> Result<Value, (ErrorCode, String)> {
        let document = self.document(uri)?;
        let folds = nomi_ide::folding_ranges(&self.vfs.snapshot(), &document.path)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&document.text, self.unit);
        let folds: Vec<Value> = (folds.into_iter())
            .map(|f| {
                let (start, end) = (text.position(f.start), text.position(f.end));
                // A branch folds from the line above it, at its column.
                let start_line = if f.line_before {
                    start.line.saturating_sub(1)
                } else {
                    start.line
                };
                let kind = match f.kind {
                    nomi_ide::FoldKind::Region => "region",
                    nomi_ide::FoldKind::Imports => "imports",
                    nomi_ide::FoldKind::Comment => "comment",
                };
                serde_json::json!({
                    "startLine": start_line, "startCharacter": start.character,
                    "endLine": end.line, "endCharacter": end.character, "kind": kind,
                })
            })
            .collect();
        Ok(Value::Array(folds))
    }

    fn selection(
        &self,
        uri: &Url,
        positions: &[lsp_types::Position],
    ) -> Result<Value, (ErrorCode, String)> {
        let document = self.document(uri)?;
        let text = Text::new(&document.text, self.unit);
        let mut found = Vec::new();
        for position in positions {
            let ranges = nomi_ide::selection_ranges(
                &self.vfs.snapshot(),
                &document.path,
                text.offset(*position),
            )
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
            // Innermost first, each the parent's child.
            let mut nested: Option<Value> = None;
            for range in ranges.iter().rev() {
                let mut selection = serde_json::json!({ "range": text.range(*range) });
                if let Some(parent) = nested.take() {
                    selection["parent"] = parent;
                }
                nested = Some(selection);
            }
            found.extend(nested);
        }
        Ok(Value::Array(found))
    }

    fn linked(&self, position: &TextDocumentPositionParams) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let ranges = nomi_ide::linked_editing(&self.vfs.snapshot(), &document.path, offset)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&document.text, self.unit);
        let ranges: Vec<lsp_types::Range> = ranges.into_iter().map(|r| text.range(r)).collect();
        Ok(serde_json::json!({ "ranges": ranges }))
    }

    fn workspace_symbols(&mut self, query: &str) -> Result<Value, (ErrorCode, String)> {
        let snapshot = self.vfs.snapshot();
        // The project of an open document, as elm-language-server has one.
        let Some(path) = self.documents.values().next().map(|d| d.path.clone()) else {
            return Ok(Value::Array(Vec::new()));
        };
        let unit = self.unit;
        let Some(index) = self.index(&snapshot, &path) else {
            return Ok(Value::Array(Vec::new()));
        };
        let mut found = Vec::new();
        for (file, symbol) in nomi_ide::workspace_symbols(&snapshot, index, query) {
            let (Ok(uri), Ok(text)) = (Url::from_file_path(&file), snapshot.read(&file)) else {
                continue;
            };
            let text = Text::new(&text, unit);
            #[allow(deprecated)] // `SymbolInformation::deprecated` must be named.
            found.push(SymbolInformation {
                name: symbol.name,
                kind: symbol_kind(symbol.kind),
                tags: None,
                deprecated: None,
                location: Location::new(uri, text.range(symbol.range)),
                container_name: None,
            });
        }
        Ok(serde_json::to_value(found).expect("symbols serialize"))
    }

    fn prepare_rename(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let (path, text) = (document.path.clone(), document.text.clone());
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &path) else {
            return Ok(Value::Null);
        };
        let range = nomi_ide::prepare_rename(&snapshot, index, &path, offset)
            .map_err(|message| (ErrorCode::RequestFailed, message.to_owned()))?;
        let text = Text::new(&text, self.unit);
        Ok(serde_json::to_value(range.map(|r| text.range(r))).expect("a range serializes"))
    }

    fn rename(
        &mut self,
        position: &TextDocumentPositionParams,
        new_name: &str,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &path) else {
            return Ok(Value::Null);
        };
        let Some(found) = nomi_ide::rename(&snapshot, index, &path, offset, new_name)
            .map_err(|message| (ErrorCode::RequestFailed, message.to_owned()))?
        else {
            return Ok(Value::Null);
        };
        // Both forms, as elm-language-server answers: `changes` for
        // clients without document changes.
        let mut changes = serde_json::Map::new();
        let mut documents = Vec::new();
        for (file, edits) in &found.files {
            let (Ok(uri), Ok(text)) = (Url::from_file_path(file), snapshot.read(file)) else {
                continue;
            };
            let text = Text::new(&text, self.unit);
            let edits: Vec<TextEdit> = (edits.iter())
                .map(|e| TextEdit::new(text.range(e.range), e.text.clone()))
                .collect();
            changes.insert(
                uri.to_string(),
                serde_json::to_value(&edits).expect("edits serialize"),
            );
            documents.push(serde_json::json!({
                "textDocument": { "uri": uri, "version": null },
                "edits": edits,
            }));
        }
        if let Some((from, to)) = &found.moved
            && let (Ok(from), Ok(to)) = (Url::from_file_path(from), Url::from_file_path(to))
        {
            documents.push(serde_json::json!({ "kind": "rename", "oldUri": from, "newUri": to }));
        }
        Ok(serde_json::json!({ "changes": changes, "documentChanges": documents }))
    }

    fn type_definition(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .and_then(|index| nomi_ide::type_definition(&snapshot, index, &path, offset));
        let response = (found.as_ref())
            .and_then(|l| self.location(&snapshot, l))
            .map(GotoDefinitionResponse::Scalar);
        Ok(serde_json::to_value(response).expect("a type definition serializes"))
    }

    fn references(
        &mut self,
        position: &TextDocumentPositionParams,
        with_declaration: bool,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .map(|index| nomi_ide::references(&snapshot, index, &path, offset, with_declaration))
            .unwrap_or_default();
        let locations: Vec<Location> = (found.iter())
            .filter_map(|l| self.location(&snapshot, l))
            .collect();
        Ok(serde_json::to_value(locations).expect("references serialize"))
    }

    fn code_lenses(&self, uri: &Url) -> Result<Value, (ErrorCode, String)> {
        let document = self.document(uri)?;
        let lenses = nomi_ide::code_lenses(&self.vfs.snapshot(), &document.path)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&document.text, self.unit);
        // The data elm-language-server gives, which comes back to resolve.
        let lenses: Vec<Value> = (lenses.into_iter())
            .map(|lens| {
                let data = match lens.kind {
                    nomi_ide::LensKind::Exposed { name, function } => serde_json::json!({
                        "codeLensType": "exposed", "uri": uri,
                        "nameNode": name, "isFunctionOrPort": function,
                    }),
                    nomi_ide::LensKind::References => {
                        serde_json::json!({ "codeLensType": "referenceCounter", "uri": uri })
                    }
                };
                serde_json::json!({ "range": text.range(lens.range), "data": data })
            })
            .collect();
        Ok(Value::Array(lenses))
    }

    fn resolve_lens(
        &mut self,
        mut lens: lsp_types::CodeLens,
    ) -> Result<Value, (ErrorCode, String)> {
        let data = lens.data.clone().unwrap_or_default();
        let uri = (data["uri"].as_str())
            .and_then(|u| Url::parse(u).ok())
            .ok_or_else(|| {
                (
                    ErrorCode::InvalidParams,
                    "a lens without its file".to_owned(),
                )
            })?;
        let path = (uri.to_file_path())
            .map_err(|()| (ErrorCode::InvalidParams, format!("{uri} isn't a file")))?;
        let snapshot = self.vfs.snapshot();
        let text = snapshot
            .read(&path)
            .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
        let text = Text::new(&text, self.unit);
        let command = match data["codeLensType"].as_str() {
            Some("exposed") => {
                let name = data["nameNode"].as_str().unwrap_or_default();
                let function = data["isFunctionOrPort"].as_bool().unwrap_or(false);
                let exposed = nomi_ide::is_exposed(&snapshot, &path, name, function)
                    .map_err(|e| (ErrorCode::InternalError, e.to_string()))?;
                let title = if exposed { "exposed" } else { "local" };
                let root = (index::manifest(&snapshot, &path))
                    .and_then(|m| m.parent().map(Path::to_path_buf))
                    .and_then(|root| Url::from_file_path(root).ok());
                match root.filter(|_| self.settings.lock().unwrap().expose_commands) {
                    Some(root) => {
                        let verb = if exposed { "unexpose" } else { "expose" };
                        let root = root.as_str().trim_end_matches('/');
                        lsp_types::Command::new(
                            title.to_owned(),
                            format!("elm.{verb}-{root}"),
                            Some(vec![serde_json::json!({ "uri": uri, "name": name })]),
                        )
                    }
                    None => lsp_types::Command::new(title.to_owned(), String::new(), None),
                }
            }
            Some("referenceCounter") => {
                let offset = text.offset(lens.range.start);
                let found = (self.index(&snapshot, &path))
                    .map(|index| nomi_ide::references(&snapshot, index, &path, offset, true))
                    .unwrap_or_default();
                let locations: Vec<Location> = (found.iter())
                    .filter_map(|l| self.location(&snapshot, l))
                    .collect();
                let title = match locations.len() {
                    1 => "1 reference".to_owned(),
                    n => format!("{n} references"),
                };
                lsp_types::Command::new(
                    title,
                    "editor.action.showReferences".to_owned(),
                    Some(vec![serde_json::json!({
                        "range": lens.range, "references": locations, "uri": uri,
                    })]),
                )
            }
            _ => return Ok(serde_json::to_value(lens).expect("a lens serializes")),
        };
        lens.command = Some(command);
        Ok(serde_json::to_value(lens).expect("a lens serializes"))
    }

    fn highlights(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .map(|index| nomi_ide::highlights(&snapshot, index, &path, offset))
            .unwrap_or_default();
        let document = self.document(&position.text_document.uri)?;
        let text = Text::new(&document.text, self.unit);
        let highlights: Vec<DocumentHighlight> = (found.into_iter())
            .map(|range| DocumentHighlight {
                range: text.range(range),
                kind: None,
            })
            .collect();
        Ok(serde_json::to_value(highlights).expect("highlights serialize"))
    }
}

impl Server {
    fn hover(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .and_then(|index| nomi_ide::hover(&snapshot, index, &path, offset));
        let response = found.map(|h| {
            let mut value = h.markdown;
            // As elm-language-server links a parameter's type: to its
            // file, at a one-based line.
            if let Some((name, path, line)) = h.link
                && let Ok(mut uri) = Url::from_file_path(&path)
            {
                uri.set_fragment(Some(&format!("L{line}")));
                value.push_str(&format!("\n\n[Go to {name}](<{uri}>)"));
            }
            lsp_types::Hover {
                contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }
        });
        Ok(serde_json::to_value(response).expect("a hover serializes"))
    }

    fn completion(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let (path, text) = (document.path.clone(), document.text.clone());
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .and_then(|index| nomi_ide::completions(&snapshot, index, &path, offset));
        let text = Text::new(&text, self.unit);
        let details = self.label_details;
        let items = |items: Vec<nomi_ide::Completion>| -> Vec<Value> {
            (items.into_iter())
                .map(|c| completion_item(c, &text, details))
                .collect()
        };
        Ok(match found {
            None => Value::Null,
            Some(nomi_ide::Completions::List(list)) => Value::Array(items(list)),
            Some(nomi_ide::Completions::Incomplete(list, incomplete)) => {
                serde_json::json!({ "isIncomplete": incomplete, "items": items(list) })
            }
        })
    }
}

/// A completion as elm-language-server writes it; its label details only
/// where the client takes them.
fn completion_item(c: nomi_ide::Completion, text: &Text, label_details: bool) -> Value {
    let mut item = serde_json::Map::new();
    if let Some(documentation) = c.documentation {
        item.insert(
            "documentation".into(),
            serde_json::json!({ "kind": "markdown", "value": documentation }),
        );
    }
    item.insert("kind".into(), serde_json::json!(c.kind as u32));
    item.insert("label".into(), serde_json::json!(c.label));
    if let Some(sort) = c.sort_text {
        item.insert("sortText".into(), serde_json::json!(sort));
    }
    if let Some(filter) = c.filter_text {
        item.insert("filterText".into(), serde_json::json!(filter));
    }
    if let Some(detail) = c.detail {
        item.insert("detail".into(), serde_json::json!(detail));
    }
    if c.preselect {
        item.insert("preselect".into(), serde_json::json!(true));
    }
    if let Some(snippet) = c.snippet {
        item.insert("insertText".into(), serde_json::json!(snippet));
        item.insert("insertTextFormat".into(), serde_json::json!(2));
    }
    if let Some(edit) = c.edit {
        let edit = TextEdit::new(text.range(edit.range), edit.text);
        item.insert(
            "textEdit".into(),
            serde_json::to_value(edit).expect("an edit serializes"),
        );
    }
    if !c.additional_edits.is_empty() {
        let edits: Vec<TextEdit> = (c.additional_edits.into_iter())
            .map(|e| TextEdit::new(text.range(e.range), e.text))
            .collect();
        item.insert(
            "additionalTextEdits".into(),
            serde_json::to_value(edits).expect("edits serialize"),
        );
    }
    let mut details = serde_json::Map::new();
    if let Some(description) = c.label_description {
        details.insert("description".into(), serde_json::json!(description));
    }
    if let Some(detail) = c.label_detail {
        details.insert("detail".into(), serde_json::json!(detail));
    }
    if label_details && !details.is_empty() {
        item.insert("labelDetails".into(), Value::Object(details));
    }
    Value::Object(item)
}

fn publish_if_current(publishing: &Mutex<()>, snapshot: &Snapshot, publish: impl FnOnce()) -> bool {
    let _held = publishing.lock().unwrap();
    if snapshot.is_stale() {
        return false;
    }
    publish();
    true
}

fn symbol_kind(kind: nomi_ide::SymbolKind) -> SymbolKind {
    match kind {
        nomi_ide::SymbolKind::Module => SymbolKind::MODULE,
        nomi_ide::SymbolKind::Function => SymbolKind::FUNCTION,
        nomi_ide::SymbolKind::Enum => SymbolKind::ENUM,
        nomi_ide::SymbolKind::Struct => SymbolKind::STRUCT,
        nomi_ide::SymbolKind::EnumMember => SymbolKind::ENUM_MEMBER,
    }
}

fn parse<P: serde::de::DeserializeOwned>(request: Request) -> Result<P, (ErrorCode, String)> {
    from(request.params).map_err(|e| (ErrorCode::InvalidParams, e.to_string()))
}

fn from<P: serde::de::DeserializeOwned>(params: Value) -> Result<P, serde_json::Error> {
    serde_json::from_value(params)
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

    #[test]
    fn positions_count_in_the_negotiated_unit() {
        let source = "a😀b\nc";
        for (unit, column) in [(ColumnUnit::Utf16, 3), (ColumnUnit::Utf8, 5)] {
            let text = Text::new(source, unit);
            let b = nomi_base::TextSize::new(5);
            assert_eq!(text.position(b), lsp_types::Position::new(0, column));
            assert_eq!(text.offset(text.position(b)), b);
            // Past the line's end is its end; past the text, the text's.
            assert_eq!(usize::from(text.offset(lsp_types::Position::new(0, 99))), 6);
            assert_eq!(usize::from(text.offset(lsp_types::Position::new(9, 0))), 8);
        }
        let range = nomi_base::TextRange::new(0.into(), 1.into());
        assert_eq!(
            Text::new(source, ColumnUnit::Utf16)
                .range(range)
                .end
                .character,
            1
        );
    }
}
