//! The requests that ask about the code: structure, navigation, hover,
//! completion, lenses and tests, and formatting's edits.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use lsp_types::{
    CodeLens, Command, CompletionItem, CompletionItemKind, CompletionItemLabelDetails,
    CompletionList, CompletionResponse, CompletionTextEdit, DocumentChangeOperation,
    DocumentChanges, DocumentHighlight, DocumentSymbolResponse, Documentation, FoldingRange,
    FoldingRangeKind, GotoDefinitionResponse, Hover, HoverContents, InsertTextFormat,
    LinkedEditingRanges, Location, MarkupContent, MarkupKind, OneOf,
    OptionalVersionedTextDocumentIdentifier, Position, PrepareRenameResponse, RenameFile,
    ResourceOp, SelectionRange, SymbolInformation, SymbolKind, TextDocumentEdit,
    TextDocumentPositionParams, TextEdit, Url, WorkspaceEdit, WorkspaceSymbolResponse,
};
use nomi_base::{TextSize, lock};
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::position::{Files, Text};
use crate::{Failure, Server, command_root, file_path};

/// What a code lens carries for its resolve, as elm-language-server
/// writes it.
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "codeLensType",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum LensData {
    Exposed {
        uri: Url,
        name_node: String,
        is_function_or_port: bool,
    },
    ReferenceCounter {
        uri: Url,
    },
}

impl Server {
    pub(crate) fn formatting(&self, uri: &Url) -> Result<Option<Vec<TextEdit>>, Failure> {
        let document = self.document(uri)?;
        let edits =
            nomi_ide::format(&self.vfs.snapshot(), &document.path).map_err(Failure::internal)?;
        let text = Text::new(&document.text, self.shared.unit);
        Ok(edits.map(|edits| edits.into_iter().map(|e| text.edit(e)).collect()))
    }

    pub(crate) fn symbols(&self, uri: &Url) -> Result<Option<DocumentSymbolResponse>, Failure> {
        let document = self.document(uri)?;
        let symbols = nomi_ide::document_symbols(&self.vfs.snapshot(), &document.path)
            .map_err(Failure::internal)?;
        let text = Text::new(&document.text, self.shared.unit);
        let symbols = (symbols.into_iter())
            .map(|s| {
                let location = Location::new(uri.clone(), text.range(s.range));
                symbol_information(s.name, s.kind, location)
            })
            .collect();
        Ok(Some(DocumentSymbolResponse::Flat(symbols)))
    }

    /// Where `find` goes from a position: a definition, or a type's.
    pub(crate) fn goto(
        &self,
        position: &TextDocumentPositionParams,
        find: fn(&Snapshot, &Index, &Path, TextSize) -> Option<nomi_ide::Location>,
    ) -> Result<Option<GotoDefinitionResponse>, Failure> {
        let at = self.at(position)?;
        let path = &at.document.path;
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, path))
            .and_then(|index| find(&snapshot, &index, path, at.offset));
        let mut files = Files::new(&snapshot, self.shared.unit);
        Ok((found.and_then(|l| files.location(&l))).map(GotoDefinitionResponse::Scalar))
    }

    pub(crate) fn folding(&self, uri: &Url) -> Result<Option<Vec<FoldingRange>>, Failure> {
        let document = self.document(uri)?;
        let folds = nomi_ide::folding_ranges(&self.vfs.snapshot(), &document.path)
            .map_err(Failure::internal)?;
        let text = Text::new(&document.text, self.shared.unit);
        let folds = (folds.into_iter())
            .map(|f| {
                let (start, end) = (text.position(f.start), text.position(f.end));
                FoldingRange {
                    // A branch folds from the line above it, at its column.
                    start_line: if f.line_before {
                        start.line.saturating_sub(1)
                    } else {
                        start.line
                    },
                    start_character: Some(start.character),
                    end_line: end.line,
                    end_character: Some(end.character),
                    kind: Some(match f.kind {
                        nomi_ide::FoldKind::Region => FoldingRangeKind::Region,
                        nomi_ide::FoldKind::Imports => FoldingRangeKind::Imports,
                        nomi_ide::FoldKind::Comment => FoldingRangeKind::Comment,
                    }),
                    collapsed_text: None,
                }
            })
            .collect();
        Ok(Some(folds))
    }

    pub(crate) fn selection(
        &self,
        uri: &Url,
        positions: &[Position],
    ) -> Result<Option<Vec<SelectionRange>>, Failure> {
        let document = self.document(uri)?;
        let text = Text::new(&document.text, self.shared.unit);
        let snapshot = self.vfs.snapshot();
        let mut found = Vec::new();
        for position in positions {
            let ranges =
                nomi_ide::selection_ranges(&snapshot, &document.path, text.offset(*position))
                    .map_err(Failure::internal)?;
            // Innermost first, each the parent's child.
            found.extend(ranges.iter().rev().fold(None, |parent, range| {
                Some(SelectionRange {
                    range: text.range(*range),
                    parent: parent.map(Box::new),
                })
            }));
        }
        Ok(Some(found))
    }

    pub(crate) fn linked(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<Option<LinkedEditingRanges>, Failure> {
        let at = self.at(position)?;
        let ranges = nomi_ide::linked_editing(&self.vfs.snapshot(), &at.document.path, at.offset)
            .map_err(Failure::internal)?;
        Ok(Some(LinkedEditingRanges {
            ranges: ranges.into_iter().map(|r| at.text.range(r)).collect(),
            word_pattern: None,
        }))
    }

    pub(crate) fn workspace_symbols(&self, query: &str) -> WorkspaceSymbolResponse {
        let snapshot = self.vfs.snapshot();
        // The project of an open document, as elm-language-server has one.
        let index = (self.documents.values().next()).and_then(|d| self.index(&snapshot, &d.path));
        let mut files = Files::new(&snapshot, self.shared.unit);
        let found = (index.iter())
            .flat_map(|index| nomi_ide::workspace_symbols(index, query))
            .filter_map(|(file, symbol)| {
                let (uri, text) = files.get(&file)?;
                let location = Location::new(uri.clone(), text.range(symbol.range));
                Some(symbol_information(symbol.name, symbol.kind, location))
            })
            .collect();
        WorkspaceSymbolResponse::Flat(found)
    }

    pub(crate) fn prepare_rename(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>, Failure> {
        let at = self.at(position)?;
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &at.document.path) else {
            return Ok(None);
        };
        let range = nomi_ide::prepare_rename(&snapshot, &index, &at.document.path, at.offset)
            .map_err(Failure::failed)?;
        Ok(range.map(|r| PrepareRenameResponse::Range(at.text.range(r))))
    }

    pub(crate) fn rename(
        &self,
        position: &TextDocumentPositionParams,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>, Failure> {
        let at = self.at(position)?;
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &at.document.path) else {
            return Ok(None);
        };
        let found = nomi_ide::rename(&snapshot, &index, &at.document.path, at.offset, new_name)
            .map_err(Failure::failed)?;
        let Some(found) = found else {
            return Ok(None);
        };
        let files = Files::new(&snapshot, self.shared.unit).edits(found.files, Text::edit);
        // Both forms, as elm-language-server answers: `changes` for
        // clients without document changes.
        let mut documents: Vec<DocumentChangeOperation> = (files.iter())
            .map(|(uri, edits)| {
                DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri: uri.clone(),
                        version: None,
                    },
                    edits: edits.iter().cloned().map(OneOf::Left).collect(),
                })
            })
            .collect();
        if let Some(moved) = &found.moved
            && let (Ok(old_uri), Ok(new_uri)) = (
                Url::from_file_path(&moved.from),
                Url::from_file_path(&moved.to),
            )
        {
            documents.push(DocumentChangeOperation::Op(ResourceOp::Rename(
                RenameFile {
                    old_uri,
                    new_uri,
                    options: None,
                    annotation_id: None,
                },
            )));
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(files.into_iter().collect()),
            document_changes: Some(DocumentChanges::Operations(documents)),
            change_annotations: None,
        }))
    }

    pub(crate) fn references(
        &self,
        position: &TextDocumentPositionParams,
        with_declaration: bool,
    ) -> Result<Option<Vec<Location>>, Failure> {
        let at = self.at(position)?;
        let declaration = if with_declaration {
            nomi_ide::Declaration::Include
        } else {
            nomi_ide::Declaration::Exclude
        };
        let snapshot = self.vfs.snapshot();
        Ok(Some(self.references_of(
            &snapshot,
            &at.document.path,
            at.offset,
            declaration,
        )))
    }

    /// The references of the name at `offset` in the file at `path`.
    fn references_of(
        &self,
        snapshot: &Snapshot,
        path: &Path,
        offset: TextSize,
        declaration: nomi_ide::Declaration,
    ) -> Vec<Location> {
        let found = (self.index(snapshot, path))
            .map(|index| nomi_ide::references(snapshot, &index, path, offset, declaration))
            .unwrap_or_default();
        let mut files = Files::new(snapshot, self.shared.unit);
        found.iter().filter_map(|l| files.location(l)).collect()
    }

    /// `elm/provideFileContents`: the text of a file of the project or of
    /// a package it uses, as the editor or the disk has it.
    pub(crate) fn file_contents(&self, params: &Value) -> Result<String, Failure> {
        let path = (params["uri"].as_str())
            .and_then(file_path)
            .ok_or_else(|| Failure::invalid("a file"))?;
        let snapshot = self.vfs.snapshot();
        let known = (self.index(&snapshot, &path))
            .is_some_and(|index| index.module(&path).is_some() || index.is_package_file(&path));
        if !known {
            let message = format!("no Elm project has {}", path.display());
            return Err(Failure::invalid(message));
        }
        snapshot.read(&path).map_err(Failure::internal)
    }

    /// `elm/findTests`: the suites of the project's test modules.
    pub(crate) fn find_tests(&self, params: &Value) -> Result<Value, Failure> {
        fn suite_json(suite: &nomi_ide::TestSuite, files: &mut Files) -> Option<Value> {
            let (uri, text) = files.get(&suite.path)?;
            let mut value = json!({
                "label": suite.label, "file": uri, "position": text.position(suite.at),
            });
            if let Some(tests) = &suite.tests {
                value["tests"] = tests.iter().filter_map(|t| suite_json(t, files)).collect();
            }
            Some(value)
        }
        let root = (params["projectFolder"].as_str())
            .and_then(file_path)
            .ok_or_else(|| Failure::invalid("a project folder"))?;
        let snapshot = self.vfs.snapshot();
        // A file of the project, for the index to start from: an open one,
        // or else its `elm.json`.
        let file = (self.documents.values())
            .map(|d| &d.path)
            .find(|p| p.starts_with(&root))
            .cloned()
            .unwrap_or_else(|| root.join("elm.json"));
        let Some(index) = self.index(&snapshot, &file) else {
            return Err(Failure::failed(format!(
                "no Elm project at {}",
                root.display()
            )));
        };
        let mut files = Files::new(&snapshot, self.shared.unit);
        let suites: Vec<Value> = (nomi_ide::find_tests(&index, &root).iter())
            .filter_map(|s| suite_json(s, &mut files))
            .collect();
        Ok(json!({ "suites": suites }))
    }

    pub(crate) fn code_lenses(&self, uri: &Url) -> Result<Option<Vec<CodeLens>>, Failure> {
        let document = self.document(uri)?;
        let lenses = nomi_ide::code_lenses(&self.vfs.snapshot(), &document.path)
            .map_err(Failure::internal)?;
        let text = Text::new(&document.text, self.shared.unit);
        let lenses = (lenses.into_iter())
            .map(|lens| {
                let uri = uri.clone();
                let data = match lens.kind {
                    nomi_ide::LensKind::Exposed { name, what } => LensData::Exposed {
                        uri,
                        name_node: name,
                        is_function_or_port: what == nomi_ide::Exposable::Value,
                    },
                    nomi_ide::LensKind::References => LensData::ReferenceCounter { uri },
                };
                CodeLens {
                    range: text.range(lens.range),
                    command: None,
                    data: Some(serde_json::to_value(data).expect("a lens's data serializes")),
                }
            })
            .collect();
        Ok(Some(lenses))
    }

    pub(crate) fn resolve_lens(&self, mut lens: CodeLens) -> Result<CodeLens, Failure> {
        let data = (lens.data.as_ref())
            .and_then(|data| LensData::deserialize(data).ok())
            .ok_or_else(|| Failure::invalid("a lens without its file"))?;
        let snapshot = self.vfs.snapshot();
        let command = match data {
            LensData::Exposed {
                uri,
                name_node,
                is_function_or_port,
            } => {
                let path = lens_file(&uri)?;
                let what = if is_function_or_port {
                    nomi_ide::Exposable::Value
                } else {
                    nomi_ide::Exposable::Type
                };
                let exposed = nomi_ide::is_exposed(&snapshot, &path, &name_node, what)
                    .map_err(Failure::internal)?;
                let title = if exposed { "exposed" } else { "local" };
                let commands = lock(&self.shared.settings)
                    .extended_capabilities
                    .expose_commands;
                match commands.then(|| command_root(&snapshot, &path)).flatten() {
                    Some(root) => {
                        let verb = if exposed { "unexpose" } else { "expose" };
                        let arguments = vec![json!({ "uri": uri, "name": name_node })];
                        Command::new(
                            title.to_owned(),
                            format!("elm.{verb}-{root}"),
                            Some(arguments),
                        )
                    }
                    None => Command::new(title.to_owned(), String::new(), None),
                }
            }
            LensData::ReferenceCounter { uri } => {
                let path = lens_file(&uri)?;
                let text = snapshot.read(&path).map_err(Failure::internal)?;
                let offset = Text::new(text, self.shared.unit).offset(lens.range.start);
                let declaration = nomi_ide::Declaration::Include;
                let locations = self.references_of(&snapshot, &path, offset, declaration);
                let title = match locations.len() {
                    1 => "1 reference".to_owned(),
                    n => format!("{n} references"),
                };
                let arguments = json!({ "range": lens.range, "references": locations, "uri": uri });
                Command::new(
                    title,
                    "editor.action.showReferences".to_owned(),
                    Some(vec![arguments]),
                )
            }
        };
        lens.command = Some(command);
        Ok(lens)
    }

    pub(crate) fn highlights(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<Option<Vec<DocumentHighlight>>, Failure> {
        let at = self.at(position)?;
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, &at.document.path))
            .map(|index| nomi_ide::highlights(&snapshot, &index, &at.document.path, at.offset))
            .unwrap_or_default();
        let highlights = (found.into_iter())
            .map(|range| DocumentHighlight {
                range: at.text.range(range),
                kind: None,
            })
            .collect();
        Ok(Some(highlights))
    }

    pub(crate) fn hover(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<Option<Hover>, Failure> {
        let at = self.at(position)?;
        let path = &at.document.path;
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, path))
            .and_then(|index| nomi_ide::hover(&snapshot, &index, path, at.offset));
        Ok(found.map(|h| {
            let mut value = h.markdown;
            // As elm-language-server links a parameter's type: to its
            // file, at a one-based line.
            if let Some(link) = h.link
                && let Ok(mut uri) = Url::from_file_path(&link.path)
            {
                uri.set_fragment(Some(&format!("L{}", link.line)));
                let _ = write!(value, "\n\n[Go to {}](<{uri}>)", link.name);
            }
            Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }
        }))
    }

    pub(crate) fn completion(
        &self,
        position: &TextDocumentPositionParams,
    ) -> Result<Option<CompletionResponse>, Failure> {
        let at = self.at(position)?;
        let path = &at.document.path;
        let snapshot = self.vfs.snapshot();
        let found = (self.index(&snapshot, path))
            .and_then(|index| nomi_ide::completions(&snapshot, &index, path, at.offset));
        let details = self.client.label_details;
        let items = |items: Vec<nomi_ide::Completion>| -> Vec<CompletionItem> {
            (items.into_iter())
                .map(|c| completion_item(c, &at.text, details))
                .collect()
        };
        Ok(found.map(|found| match found {
            nomi_ide::Completions::Items(list) => CompletionResponse::Array(items(list)),
            nomi_ide::Completions::List {
                items: list,
                incomplete,
            } => CompletionResponse::List(CompletionList {
                is_incomplete: incomplete,
                items: items(list),
            }),
        }))
    }
}

/// The file a lens names.
fn lens_file(uri: &Url) -> Result<PathBuf, Failure> {
    uri.to_file_path()
        .map_err(|()| Failure::invalid(format!("{uri} isn't a file")))
}

/// A completion as elm-language-server writes it; its label details only
/// where the client takes them.
fn completion_item(c: nomi_ide::Completion, text: &Text, label_details: bool) -> CompletionItem {
    let details = CompletionItemLabelDetails {
        detail: c.label_detail,
        description: c.label_description,
    };
    let documentation = c.documentation.map(|value| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        })
    });
    CompletionItem {
        label: c.label,
        label_details: (label_details
            && (details.detail.is_some() || details.description.is_some()))
        .then_some(details),
        kind: Some(completion_kind(c.kind)),
        detail: c.detail,
        documentation,
        preselect: c.preselect.then_some(true),
        sort_text: c.sort_text,
        filter_text: c.filter_text,
        insert_text_format: c.snippet.is_some().then_some(InsertTextFormat::SNIPPET),
        insert_text: c.snippet,
        text_edit: c.edit.map(|e| CompletionTextEdit::Edit(text.edit(e))),
        additional_text_edits: (!c.additional_edits.is_empty()).then(|| {
            c.additional_edits
                .into_iter()
                .map(|e| text.edit(e))
                .collect()
        }),
        ..CompletionItem::default()
    }
}

/// The protocol's number for a completion's kind.
fn completion_kind(kind: nomi_ide::CompletionKind) -> CompletionItemKind {
    match kind {
        nomi_ide::CompletionKind::Text => CompletionItemKind::TEXT,
        nomi_ide::CompletionKind::Function => CompletionItemKind::FUNCTION,
        nomi_ide::CompletionKind::Field => CompletionItemKind::FIELD,
        nomi_ide::CompletionKind::Variable => CompletionItemKind::VARIABLE,
        nomi_ide::CompletionKind::Module => CompletionItemKind::MODULE,
        nomi_ide::CompletionKind::Enum => CompletionItemKind::ENUM,
        nomi_ide::CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        nomi_ide::CompletionKind::Snippet => CompletionItemKind::SNIPPET,
        nomi_ide::CompletionKind::EnumMember => CompletionItemKind::ENUM_MEMBER,
        nomi_ide::CompletionKind::Struct => CompletionItemKind::STRUCT,
        nomi_ide::CompletionKind::Operator => CompletionItemKind::OPERATOR,
    }
}

#[expect(deprecated, reason = "`SymbolInformation::deprecated` must be named")]
fn symbol_information(
    name: String,
    kind: nomi_ide::SymbolKind,
    location: Location,
) -> SymbolInformation {
    let kind = match kind {
        nomi_ide::SymbolKind::Module => SymbolKind::MODULE,
        nomi_ide::SymbolKind::Function => SymbolKind::FUNCTION,
        nomi_ide::SymbolKind::Enum => SymbolKind::ENUM,
        nomi_ide::SymbolKind::Struct => SymbolKind::STRUCT,
        nomi_ide::SymbolKind::EnumMember => SymbolKind::ENUM_MEMBER,
    };
    SymbolInformation {
        name,
        kind,
        tags: None,
        deprecated: None,
        location,
        container_name: None,
    }
}
