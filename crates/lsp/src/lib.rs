//! `nomi lsp`: the Language Server Protocol over the engine (architecture
//! §9). This crate holds the protocol, positions and the order of work;
//! what the answers are is `nomi_ide`'s. The editor's text lies over the
//! disk in the engine's VFS, and diagnostics are worked out on the pool
//! against a snapshot and published only while it is current (CR.2).

mod position;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    Completion, DocumentHighlightRequest, DocumentSymbolRequest, Formatting, GotoDefinition,
    HoverRequest, References, Request as _,
};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentHighlight,
    DocumentHighlightParams, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, InitializeParams, Location, OneOf, PublishDiagnosticsParams,
    ReferenceParams, ServerCapabilities, ServerInfo, SymbolInformation, SymbolKind,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
};
use nomi_base::{ColumnUnit, TextSize};
use nomi_engine::index::{self, Index, Memo};
use nomi_engine::paths::Cache;
use nomi_engine::{Pool, Revision, Snapshot, Store, Vfs};
use serde_json::Value;

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
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        document_highlight_provider: Some(OneOf::Left(true)),
        hover_provider: Some(lsp_types::HoverProviderCapability::Simple(true)),
        completion_provider: Some(lsp_types::CompletionOptions {
            trigger_characters: Some(vec![".".to_owned()]),
            ..lsp_types::CompletionOptions::default()
        }),
        ..ServerCapabilities::default()
    };
    let result = serde_json::json!({
        "capabilities": capabilities,
        "serverInfo": ServerInfo {
            name: "nomi".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    });
    connection.initialize_finish(id, result)?;
    let mut server = Server {
        unit,
        vfs: Arc::new(Vfs::new()),
        documents: HashMap::new(),
        publishing: Arc::new(Mutex::new(())),
        sender: connection.sender.clone(),
        store,
        memo,
        index: None,
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
            Message::Response(_) => {}
        }
    }
    Ok(())
}

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
    /// The packages' interfaces, made once per server.
    memo: Memo,
    /// The last project index, its revision and its `elm.json`: indexing
    /// a project takes a while, and navigation asks again and again
    /// between edits.
    index: Option<(Revision, PathBuf, Index)>,
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
            References::METHOD => parse(request).and_then(|p: ReferenceParams| {
                self.references(&p.text_document_position, p.context.include_declaration)
            }),
            HoverRequest::METHOD => parse(request)
                .and_then(|p: lsp_types::HoverParams| self.hover(&p.text_document_position_params)),
            Completion::METHOD => parse(request).and_then(|p: lsp_types::CompletionParams| {
                self.completion(&p.text_document_position)
            }),
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
            _ => {}
        }
    }

    fn open(&mut self, uri: Url, text: String) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        self.documents.insert(uri, Document { path, text });
        self.revise();
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
        self.revise();
    }

    fn close(&mut self, uri: &Url) {
        if let Some(document) = self.documents.remove(uri) {
            {
                let _held = self.publishing.lock().unwrap();
                let _ = self.vfs.remove_overlay(&document.path);
            }
            self.revise();
            let params = PublishDiagnosticsParams::new(uri.clone(), Vec::new(), None);
            send(&self.sender, notification::<PublishDiagnostics>(params));
        }
    }

    /// Lays the open documents over the disk as a new revision, then works
    /// out every open document's diagnostics against it: any change can
    /// leave another document's diagnostics unpublished.
    fn revise(&mut self) {
        let snapshot = {
            let _held = self.publishing.lock().unwrap();
            let overlays: Vec<_> = self.documents.values().collect();
            for document in &overlays {
                let _ = self.vfs.set_overlay(&document.path, document.text.as_str());
            }
            self.vfs.snapshot()
        };
        for (uri, document) in &self.documents {
            let (uri, path, unit) = (uri.clone(), document.path.clone(), self.unit);
            let (snapshot, publishing) = (snapshot.clone(), self.publishing.clone());
            let sender = self.sender.clone();
            Pool::global().spawn(move || {
                let Ok(found) = nomi_ide::diagnostics(&snapshot, &path) else {
                    return;
                };
                let Ok(text) = snapshot.read(&path) else {
                    return;
                };
                let text = Text::new(&text, unit);
                let diagnostics = found
                    .into_iter()
                    .map(|d| Diagnostic {
                        range: text.range(d.range),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("nomi".to_owned()),
                        message: format!("{}\n\n{}", d.title, d.message),
                        ..Diagnostic::default()
                    })
                    .collect();
                let params = PublishDiagnosticsParams::new(uri, diagnostics, None);
                publish_if_current(&publishing, &snapshot, || {
                    send(&sender, notification::<PublishDiagnostics>(params));
                });
            });
        }
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
            let index = index::index(snapshot, &self.store, path, &mut self.memo)?;
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
        let found =
            (self.index(&snapshot, &path)).and_then(|index| nomi_ide::hover(index, &path, offset));
        let document = self.document(&position.text_document.uri)?;
        let text = Text::new(&document.text, self.unit);
        let response = found.map(|h| lsp_types::Hover {
            contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                kind: lsp_types::MarkupKind::Markdown,
                value: h.markdown,
            }),
            range: Some(text.range(h.range)),
        });
        Ok(serde_json::to_value(response).expect("a hover serializes"))
    }

    fn completion(
        &mut self,
        position: &TextDocumentPositionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let (document, offset) = self.at(position)?;
        let path = document.path.clone();
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &path))
            .map(|index| nomi_ide::completions(index, &path, offset))
            .unwrap_or_default();
        let items: Vec<lsp_types::CompletionItem> = (found.into_iter())
            .map(|c| lsp_types::CompletionItem {
                label: c.label,
                kind: Some(match c.kind {
                    nomi_ide::CompletionKind::Value => lsp_types::CompletionItemKind::VALUE,
                    nomi_ide::CompletionKind::Type => lsp_types::CompletionItemKind::CLASS,
                    nomi_ide::CompletionKind::Constructor => {
                        lsp_types::CompletionItemKind::ENUM_MEMBER
                    }
                    nomi_ide::CompletionKind::Module => lsp_types::CompletionItemKind::MODULE,
                }),
                detail: c.detail,
                ..lsp_types::CompletionItem::default()
            })
            .collect();
        Ok(serde_json::to_value(items).expect("completions serialize"))
    }
}

/// Runs `publish` if `snapshot` is still the latest revision, holding
/// `publishing` so that no newer one is made meanwhile. Whether it ran.
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
