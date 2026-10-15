//! The requests that answer with edits: code actions and the commands
//! they run, file events, and moving functions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::request::{ApplyWorkspaceEdit, Request, ShowMessageRequest};
use lsp_types::{
    ApplyWorkspaceEditParams, ApplyWorkspaceEditResponse, CodeActionKind, CodeActionParams,
    Command, CreateFilesParams, Diagnostic, DiagnosticSeverity, DiagnosticTag,
    ExecuteCommandParams, MessageActionItem, MessageType, Position, Range, RenameFilesParams,
    ShowMessageRequestParams, TextEdit, Url, WorkspaceEdit,
};
use nomi_base::{TextSize, lock};
use nomi_engine::console::{Captured, Personality};
use nomi_engine::index::PackageModules;
use nomi_engine::paths::Cache;
use nomi_engine::{Revision, Snapshot, Vfs};
use nomi_ide::{
    Action, ActionCommand, ActionData, ActionDiagnostics, ActionKind, Diagnosed, DiagnosticCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::diagnostics::{Cause, DiagnosticData, severity};
use crate::position::{Files, Text};
use crate::{Failure, Server, command_root, file_path, project_root};

/// The command an install action runs.
pub(crate) const INSTALL_PACKAGE: &str = "nomi.installPackage";

/// A package to install in a project, once the user agrees.
pub(crate) struct Install {
    pub(crate) root: PathBuf,
    pub(crate) package: String,
    /// What the first run printed, which asked the user.
    pub(crate) printed: String,
}

/// How an install's question is answered.
#[derive(Clone, Copy)]
pub(crate) enum Answer {
    Yes,
    /// Not at all: the install stops at its question.
    Unanswered,
}

/// The choices an install's question offers.
#[derive(Clone, Copy)]
pub(crate) enum Choice {
    Yes,
    No,
}

impl Choice {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Choice::Yes => "Yes",
            Choice::No => "No",
        }
    }
}

/// What a refactor action carries for its resolve, as elm-language-server
/// writes it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefactorData {
    action_name: String,
    refactor_name: String,
    range: Range,
    uri: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename_position: Option<Position>,
}

/// `workspace/applyEdit` marked as a refactoring, as elm-language-server
/// sends a move; lsp-types has no field for the mark.
enum ApplyRefactoring {}

impl Request for ApplyRefactoring {
    type Params = Refactoring;
    type Result = ApplyWorkspaceEditResponse;
    const METHOD: &'static str = ApplyWorkspaceEdit::METHOD;
}

#[derive(Serialize, Deserialize)]
struct Refactoring {
    label: String,
    edit: WorkspaceEdit,
    metadata: RefactoringMetadata,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefactoringMetadata {
    is_refactoring: bool,
}

/// What an install did: whether it asked its question, and what it
/// printed.
pub(crate) struct Installed {
    asked: bool,
    pub(crate) printed: String,
}

impl Server {
    /// `workspace/executeCommand`: the install a code action offers,
    /// asked about first unless the settings say not to, as
    /// elm-language-server runs `elm install`.
    pub(crate) fn execute(
        &mut self,
        params: &ExecuteCommandParams,
    ) -> Result<Option<Value>, Failure> {
        if params.command != INSTALL_PACKAGE {
            return Err(Failure::invalid(format!("no command {}", params.command)));
        }
        let (Some(uri), Some(package)) = (
            params.arguments.first().and_then(Value::as_str),
            params.arguments.get(1).and_then(Value::as_str),
        ) else {
            return Err(Failure::invalid("a file and a package"));
        };
        let root = file_path(uri)
            .and_then(|path| project_root(&self.vfs.snapshot(), &path))
            .ok_or_else(|| Failure::invalid(format!("{uri} has no project")))?;
        if lock(&self.shared.settings).skip_install_confirmation {
            self.install(&root, package, Answer::Yes);
            return Ok(None);
        }
        let Installed { asked, printed } = self.install(&root, package, Answer::Unanswered);
        if !asked {
            if !printed.trim().is_empty() {
                self.show(MessageType::INFO, printed.trim());
            }
            return Ok(None);
        }
        let actions = [Choice::Yes, Choice::No].map(|choice| MessageActionItem {
            title: choice.title().to_owned(),
            properties: HashMap::new(),
        });
        let id = self.ask::<ShowMessageRequest>(ShowMessageRequestParams {
            typ: MessageType::INFO,
            message: printed.replace("[Y/n]:", "").trim().to_owned(),
            actions: Some(actions.into()),
        });
        let package = package.to_owned();
        self.installs.insert(
            id,
            Install {
                root,
                package,
                printed,
            },
        );
        Ok(None)
    }

    /// Runs `elm install package` in `root` as nomi does it, its question
    /// answered as `answer` says. What it writes is a new revision,
    /// checked as an edit is, with the store's packages read again.
    pub(crate) fn install(&mut self, root: &Path, package: &str, answer: Answer) -> Installed {
        let Ok(cache) = Cache::locate() else {
            let printed = "nomi has no cache directory for packages.".to_owned();
            return Installed {
                asked: false,
                printed,
            };
        };
        let mut console = Captured::default();
        if let Answer::Yes = answer {
            console.lines.push_back(Ok("y".to_owned()));
        }
        // A VFS of its own reads the disk's `elm.json`, not the editor's,
        // and makes a revision only if it writes.
        let vfs = Vfs::new();
        let cx = nomi_engine::Context {
            vfs: &vfs,
            dir: root,
            cache: &cache,
            http: &nomi_engine::Web::new(),
        };
        nomi_engine::install::run(&cx, &mut console, Personality::Elm, Some(package));
        if vfs.revision() != Revision::default() {
            self.packages.take();
            self.revise(Cause::Edited(root.join("elm.json")), |vfs| {
                vfs.disk_changed();
            });
        }
        // Its standard error, if any, is written last; but a run left
        // unanswered ends with what a missing answer says, which isn't the
        // question's.
        let printed = match answer {
            Answer::Unanswered if console.asked => console.out,
            Answer::Unanswered | Answer::Yes => console.out + &console.err,
        };
        Installed {
            asked: console.asked,
            printed,
        }
    }

    /// `codeAction/resolve`: a refactor's edits, computed now.
    pub(crate) fn resolve_action(&self, mut action: Value) -> Result<Value, Failure> {
        let Ok(data) = RefactorData::deserialize(&action["data"]) else {
            return Ok(action);
        };
        let document = self.document(&data.uri)?;
        let text = Text::new(&document.text, self.shared.unit);
        let range = text.text_range(data.range);
        let Some(refactor) = nomi_ide::RefactorAction::parse(&data.action_name) else {
            return Ok(action);
        };
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &document.path) else {
            return Ok(action);
        };
        if let Some((edits, rename)) =
            nomi_ide::refactor_edits(&index, &snapshot, &document.path, refactor, range)
        {
            let edits = edits.into_iter().map(|e| text.edit(e)).collect();
            let edit = WorkspaceEdit::new(HashMap::from([(data.uri, edits)]));
            action["edit"] = serde_json::to_value(edit).expect("an edit serializes");
            if let Some(rename) = rename {
                action["data"]["renamePosition"] = json!(rename_position(&text, rename));
            }
        }
        Ok(action)
    }

    /// `elm/expose` and `elm/unexpose`: the exposing list's edit, applied
    /// by the client.
    pub(crate) fn expose(
        &mut self,
        params: &Value,
        exposure: nomi_ide::Exposure,
    ) -> Result<Value, Failure> {
        let (Some(uri), Some(name)) = (params["uri"].as_str(), params["name"].as_str()) else {
            return Err(Failure::invalid("a file and a name"));
        };
        let uri = Url::parse(uri).map_err(Failure::invalid)?;
        let document = self.document(&uri)?;
        let edit = nomi_ide::exposing_edit(&self.vfs.snapshot(), &document.path, name, exposure);
        if let Some(edit) = edit {
            let edit = Text::new(&document.text, self.shared.unit).edit(edit);
            self.ask::<ApplyWorkspaceEdit>(ApplyWorkspaceEditParams {
                label: None,
                edit: WorkspaceEdit::new(HashMap::from([(uri, vec![edit])])),
            });
        }
        Ok(Value::Null)
    }

    /// `workspace/didCreateFiles`: a header for each new, empty module,
    /// applied by the client in one edit.
    pub(crate) fn created_files(&mut self, params: &CreateFilesParams) {
        let snapshot = self.vfs.snapshot();
        let mut changes = HashMap::new();
        for file in &params.files {
            let Ok(uri) = Url::parse(&file.uri) else {
                continue;
            };
            let header = (uri.to_file_path().ok())
                .and_then(|path| nomi_ide::created_module_header(&snapshot, &path));
            if let Some(header) = header {
                changes.insert(uri, vec![TextEdit::new(Range::default(), header.text)]);
            }
        }
        self.ask::<ApplyWorkspaceEdit>(ApplyWorkspaceEditParams {
            label: None,
            edit: WorkspaceEdit::new(changes),
        });
    }

    /// `workspace/willRenameFiles`: each module renamed after its new
    /// place, wherever it's named.
    pub(crate) fn rename_files(&self, params: &RenameFilesParams) -> WorkspaceEdit {
        let snapshot = self.vfs.snapshot();
        let mut files = Files::new(&snapshot, self.shared.unit);
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for file in &params.files {
            let (Some(old), Some(new)) = (file_path(&file.old_uri), file_path(&file.new_uri))
            else {
                continue;
            };
            let Some(index) = self.index(&snapshot, &old) else {
                continue;
            };
            let renamed = nomi_ide::renamed_files(&snapshot, &index, &old, &new);
            for (uri, edits) in files.edits(renamed, Text::edit) {
                changes.entry(uri).or_default().extend(edits);
            }
        }
        WorkspaceEdit::new(changes)
    }

    /// The file a move request is about, and where in it.
    fn move_source(&self, snapshot: &Snapshot, params: &Value) -> Option<(PathBuf, TextSize)> {
        let path = file_path(params["sourceUri"].as_str()?)?;
        let position = Position::deserialize(&params["params"]["range"]["start"]).ok()?;
        let offset = Text::new(snapshot.read(&path).ok()?, self.shared.unit).offset(position);
        Some((path, offset))
    }

    /// `elm/getMoveDestinations`: the project's other modules.
    pub(crate) fn move_destinations(&self, params: &Value) -> Value {
        let snapshot = self.vfs.snapshot();
        let destinations: Vec<Value> = (self.move_source(&snapshot, params))
            .and_then(|(path, _)| {
                let root = project_root(&snapshot, &path)?;
                let index = self.index(&snapshot, &path)?;
                Some(nomi_ide::move_destinations(&index, &root, &path))
            })
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let uri = Url::from_file_path(&d.path).ok()?;
                Some(json!({ "name": d.name, "path": d.folder, "uri": uri }))
            })
            .collect();
        json!({ "destinations": destinations })
    }

    /// `elm/move`: the function moved, applied by the client as one
    /// refactoring.
    pub(crate) fn move_to(&mut self, params: &Value) -> Value {
        let snapshot = self.vfs.snapshot();
        let destination = params["destination"]["uri"].as_str().and_then(file_path);
        let (Some((path, offset)), Some(destination)) =
            (self.move_source(&snapshot, params), destination)
        else {
            return Value::Null;
        };
        let Some(files) = (self.index(&snapshot, &path)).and_then(|index| {
            nomi_ide::move_function(&snapshot, &index, &path, offset, &destination)
        }) else {
            return Value::Null;
        };
        let changes = (Files::new(&snapshot, self.shared.unit))
            .edits(files, |text, edit| match edit {
                nomi_ide::MoveEdit::Edit(e) => text.edit(e),
                nomi_ide::MoveEdit::AtLine { line, text } => {
                    let at = Position::new(line, 0);
                    TextEdit::new(Range::new(at, at), text)
                }
            })
            .into_iter()
            .collect();
        self.ask::<ApplyRefactoring>(Refactoring {
            label: "Move Function".to_owned(),
            edit: WorkspaceEdit::new(changes),
            metadata: RefactoringMetadata {
                is_refactoring: true,
            },
        });
        Value::Null
    }

    pub(crate) fn code_actions(&self, params: &CodeActionParams) -> Result<Value, Failure> {
        let uri = &params.text_document.uri;
        let document = self.document(uri)?;
        let path = &document.path;
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, path) else {
            return Ok(Value::Array(Vec::new()));
        };
        let text = Text::new(&document.text, self.shared.unit);
        let client_diagnostics = &params.context.diagnostics;
        let given: Vec<Diagnosed> = (client_diagnostics.iter())
            .map(|d| diagnosed(d, &text))
            .collect();
        let problems = lock(&self.shared.latest)
            .get(path)
            .cloned()
            .unwrap_or_default();
        let moving = lock(&self.shared.settings)
            .extended_capabilities
            .move_function;
        let request = nomi_ide::Request {
            range: text.text_range(params.range),
            given: &given,
            problems: &problems,
            packages: (self.packages).get_or_init(|| PackageModules::read(&self.shared.store)),
            resolve: if self.client.resolves_edits {
                nomi_ide::Resolve::Later
            } else {
                nomi_ide::Resolve::Now
            },
            moving: if moving {
                nomi_ide::Moving::Supported
            } else {
                nomi_ide::Moving::Unsupported
            },
        };
        let actions = nomi_ide::code_actions(&snapshot, &index, path, &request);
        let mut files = Files::new(&snapshot, self.shared.unit);
        let asked = Asked {
            uri,
            path,
            text: &text,
            snapshot: &snapshot,
            client_diagnostics,
        };
        Ok((actions.into_iter())
            .map(|action| action_json(action, asked, &mut files))
            .collect())
    }
}

/// What a code action request asked about, which its actions name: the
/// file, its text and the diagnostics the client sent.
#[derive(Clone, Copy)]
struct Asked<'a> {
    uri: &'a Url,
    path: &'a Path,
    text: &'a Text<'a>,
    snapshot: &'a Snapshot,
    client_diagnostics: &'a [Diagnostic],
}

/// A diagnostic the client sent, as code actions read it.
fn diagnosed(d: &Diagnostic, text: &Text) -> Diagnosed {
    let data = (d.data.as_ref())
        .and_then(|data| DiagnosticData::deserialize(data).ok())
        .unwrap_or_default();
    Diagnosed {
        code: match data.code.as_str() {
            "" => DiagnosticCode::Missing,
            code => {
                nomi_ide::Code::parse(code).map_or(DiagnosticCode::Other, DiagnosticCode::Known)
            }
        },
        message: d.message.clone(),
        range: text.text_range(d.range),
        severity: match d.severity {
            Some(DiagnosticSeverity::ERROR) => nomi_ide::Severity::Error,
            Some(DiagnosticSeverity::INFORMATION | DiagnosticSeverity::HINT) => {
                nomi_ide::Severity::Information
            }
            _ => nomi_ide::Severity::Warning,
        },
        source: d.source.as_deref().and_then(nomi_ide::Source::parse),
        unnecessary: (d.tags.iter().flatten()).any(|t| *t == DiagnosticTag::UNNECESSARY),
        // elm-review's fixes, as its diagnostic carries them.
        fixes: (data.fixes.into_iter().flatten())
            .map(|fix| nomi_ide::Edit {
                range: text.text_range(fix.range),
                text: fix.string,
            })
            .collect(),
    }
}

/// A code action as elm-language-server writes it. Its diagnostics stay
/// JSON: the program's carry a `uri` the protocol's type has no room for.
fn action_json(action: Action, asked: Asked<'_>, files: &mut Files) -> Value {
    let Asked { uri, text, .. } = asked;
    let (data, refactored) = match action.data {
        ActionData::Fix { id, all } => {
            let mut fix = json!({ "fixId": id.as_str() });
            if all {
                fix["isFixAll"] = true.into();
            }
            (Some(fix), None)
        }
        ActionData::Refactor {
            action: refactor,
            range,
            rename,
        } => {
            let range = text.range(range);
            let data = RefactorData {
                action_name: refactor.as_str().to_owned(),
                refactor_name: refactor.refactor().to_owned(),
                range,
                uri: uri.clone(),
                rename_position: rename.map(|rename| rename_position(text, rename)),
            };
            let data = serde_json::to_value(data).expect("a refactor's data serializes");
            (Some(data), Some(range))
        }
        ActionData::None => (None, None),
    };
    // A move is offered only as a refactor, whose range it moves from.
    let command = match &action.command {
        None => None,
        Some(ActionCommand::InstallPackage { package }) => Some(Command::new(
            "Install Package".to_owned(),
            INSTALL_PACKAGE.to_owned(),
            Some(vec![json!(uri), json!(package)]),
        )),
        Some(ActionCommand::MoveFunction { name }) => refactored.map(|range| {
            let arguments = vec![
                json!("moveFunction"),
                json!({ "textDocument": { "uri": uri }, "range": range }),
                json!(name),
            ];
            let root = command_root(asked.snapshot, asked.path).unwrap_or_default();
            Command::new(
                "Refactor".to_owned(),
                format!("elm.refactor-{root}"),
                Some(arguments),
            )
        }),
    };
    let kind = match action.kind {
        ActionKind::QuickFix => CodeActionKind::QUICKFIX,
        ActionKind::Refactor => CodeActionKind::REFACTOR,
        ActionKind::RefactorExtract => CodeActionKind::REFACTOR_EXTRACT,
        ActionKind::RefactorMove => CodeActionKind::new("refactor.move"),
    };
    let edit = (action.edits)
        .map(|edits| WorkspaceEdit::new(files.edits(edits, Text::edit).into_iter().collect()));
    let mut json = serde_json::to_value(lsp_types::CodeAction {
        title: action.title,
        kind: Some(kind),
        edit,
        command,
        is_preferred: action.preferred,
        data,
        ..lsp_types::CodeAction::default()
    })
    .expect("a code action serializes");
    match action.diagnostics {
        Some(ActionDiagnostics::Given(i)) => {
            json["diagnostics"] = json!([asked.client_diagnostics[i]]);
        }
        Some(ActionDiagnostics::Program(diagnostics)) => {
            let diagnostics: Vec<Value> = (diagnostics.iter())
                .map(|d| {
                    let code = match d.code {
                        DiagnosticCode::Known(code) => code.as_str(),
                        DiagnosticCode::Other | DiagnosticCode::Missing => "",
                    };
                    let mut json = json!({
                        "code": code, "message": d.message, "range": text.range(d.range),
                        "severity": severity(d.severity),
                        "source": d.source.map(nomi_ide::Source::as_str), "uri": uri,
                    });
                    if d.unnecessary {
                        json["tags"] = json!([DiagnosticTag::UNNECESSARY]);
                    }
                    json
                })
                .collect();
            json["diagnostics"] = diagnostics.into();
        }
        None => {}
    }
    json
}

/// A refactor's rename position, where the client starts renaming.
fn rename_position(text: &Text, rename: nomi_ide::RenameAt) -> Position {
    let at = text.position(rename.from);
    Position::new(at.line + rename.lines, at.character + rename.columns)
}
