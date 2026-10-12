//! The requests that answer with edits: code actions and the commands
//! they run, file events, and moving functions.

use super::*;

/// A package to install in a project, once the user agrees.
pub(crate) struct Install {
    pub root: PathBuf,
    pub package: String,
    /// What the first run printed, up to its question.
    pub asked: String,
}

impl Server {
    /// `workspace/executeCommand`: the install a code action offers,
    /// asked about first unless the settings say not to, as
    /// elm-language-server runs `elm install`.
    pub(crate) fn execute(
        &mut self,
        params: &lsp_types::ExecuteCommandParams,
    ) -> Result<Value, (ErrorCode, String)> {
        if params.command != INSTALL_PACKAGE {
            return Err((
                ErrorCode::InvalidParams,
                format!("no command {}", params.command),
            ));
        }
        let (Some(uri), Some(package)) = (
            params.arguments.first().and_then(Value::as_str),
            params.arguments.get(1).and_then(Value::as_str),
        ) else {
            return Err((ErrorCode::InvalidParams, "a file and a package".to_owned()));
        };
        let root = (Url::parse(uri).ok())
            .and_then(|u| u.to_file_path().ok())
            .and_then(|path| index::manifest(&self.vfs.snapshot(), &path))
            .and_then(|m| m.parent().map(Path::to_path_buf))
            .ok_or_else(|| (ErrorCode::InvalidParams, format!("{uri} has no project")))?;
        if self.settings.lock().unwrap().skip_install_confirmation {
            install_package(&root, package, Some("y"));
            return Ok(Value::Null);
        }
        let (asked, printed) = install_package(&root, package, None);
        if !asked {
            if !printed.trim().is_empty() {
                self.show(lsp_types::MessageType::INFO, printed.trim());
            }
            return Ok(Value::Null);
        }
        let id = lsp_server::RequestId::from(format!("nomi/install/{package}"));
        let question = serde_json::json!({
            "type": lsp_types::MessageType::INFO,
            "message": printed.replace("[Y/n]:", "").trim(),
            "actions": [{ "title": "Yes" }, { "title": "No" }],
        });
        self.installs.insert(
            id.clone(),
            Install {
                root,
                package: package.to_owned(),
                asked: printed,
            },
        );
        let request = Request::new(id, "window/showMessageRequest".to_owned(), question);
        send(&self.sender, Message::Request(request));
        Ok(Value::Null)
    }

    /// `codeAction/resolve`: a refactor's edits, computed now.
    pub(crate) fn resolve_action(
        &mut self,
        mut action: Value,
    ) -> Result<Value, (ErrorCode, String)> {
        let data = action["data"].clone();
        let (Some(uri), Some(name)) = (data["uri"].as_str(), data["actionName"].as_str()) else {
            return Ok(action);
        };
        let uri = Url::parse(uri).map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?;
        let document = self.document(&uri)?;
        let (path, text) = (document.path.clone(), document.text.clone());
        let text = Text::new(&text, self.unit);
        let range: lsp_types::Range = serde_json::from_value(data["range"].clone())
            .map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?;
        let range = text.text_range(range);
        let snapshot = self.vfs.snapshot();
        let Some(index) = self.index(&snapshot, &path) else {
            return Ok(action);
        };
        if let Some((edits, rename)) =
            nomi_ide::refactor_edits(index, &snapshot, &path, name, range)
        {
            let edits: Vec<TextEdit> = (edits.iter())
                .map(|e| TextEdit::new(text.range(e.range), e.text.clone()))
                .collect();
            action["edit"] = serde_json::json!({ "changes": { uri.as_str(): edits } });
            if let Some(rename) = rename {
                action["data"]["renamePosition"] =
                    serde_json::json!(rename_position(&text, rename));
            }
        }
        Ok(action)
    }

    /// `elm/expose` and `elm/unexpose`: the exposing list's edit, applied
    /// by the client.
    pub(crate) fn expose(
        &mut self,
        params: &Value,
        expose: bool,
    ) -> Result<Value, (ErrorCode, String)> {
        let (Some(uri), Some(name)) = (params["uri"].as_str(), params["name"].as_str()) else {
            return Err((ErrorCode::InvalidParams, "a file and a name".to_owned()));
        };
        let uri = Url::parse(uri).map_err(|e| (ErrorCode::InvalidParams, e.to_string()))?;
        let document = self.document(&uri)?;
        let path = document.path.clone();
        let text = Text::new(&document.text, self.unit);
        let snapshot = self.vfs.snapshot();
        if let Some(edit) = nomi_ide::exposing_edit(&snapshot, &path, name, expose) {
            let edit = TextEdit::new(text.range(edit.range), edit.text);
            let id = lsp_server::RequestId::from(format!("nomi/expose/{name}"));
            let params = serde_json::json!({ "edit": { "changes": { uri.as_str(): [edit] } } });
            let request = Request::new(id, "workspace/applyEdit".to_owned(), params);
            send(&self.sender, Message::Request(request));
        }
        Ok(Value::Null)
    }

    /// `workspace/didCreateFiles`: a header for each new, empty module,
    /// applied by the client in one edit.
    pub(crate) fn created_files(&mut self, params: &lsp_types::CreateFilesParams) {
        let snapshot = self.vfs.snapshot();
        let mut changes = serde_json::Map::new();
        for file in &params.files {
            let Some(path) = Url::parse(&file.uri)
                .ok()
                .and_then(|u| u.to_file_path().ok())
            else {
                continue;
            };
            if let Some(edit) = nomi_ide::created_module_header(&snapshot, &path) {
                let edit = TextEdit::new(lsp_types::Range::default(), edit.text);
                changes.insert(file.uri.clone(), serde_json::json!([edit]));
            }
        }
        let id = lsp_server::RequestId::from("nomi/created".to_owned());
        let params = serde_json::json!({ "edit": { "changes": changes } });
        send(
            &self.sender,
            Message::Request(Request::new(id, "workspace/applyEdit".to_owned(), params)),
        );
    }

    /// `workspace/willRenameFiles`: each module renamed after its new
    /// place, wherever it's named.
    pub(crate) fn rename_files(
        &mut self,
        params: &lsp_types::RenameFilesParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let snapshot = self.vfs.snapshot();
        let unit = self.unit;
        let mut changes = serde_json::Map::new();
        for file in &params.files {
            let path = |uri: &str| Url::parse(uri).ok().and_then(|u| u.to_file_path().ok());
            let (Some(old), Some(new)) = (path(&file.old_uri), path(&file.new_uri)) else {
                continue;
            };
            let Some(index) = self.index(&snapshot, &old) else {
                continue;
            };
            for (renamed, edits) in nomi_ide::renamed_files(&snapshot, index, &old, &new) {
                let (Ok(uri), Ok(text)) = (Url::from_file_path(&renamed), snapshot.read(&renamed))
                else {
                    continue;
                };
                let text = Text::new(&text, unit);
                let edits: Vec<TextEdit> = (edits.iter())
                    .map(|e| TextEdit::new(text.range(e.range), e.text.clone()))
                    .collect();
                let entry = changes
                    .entry(uri.to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(existing) = entry {
                    existing.extend(edits.into_iter().map(|e| serde_json::json!(e)));
                }
            }
        }
        Ok(serde_json::json!({ "changes": changes }))
    }

    /// The file a move request is about, and where in it.
    pub(crate) fn move_source(&mut self, params: &Value) -> Option<(PathBuf, TextSize)> {
        let uri = Url::parse(params["sourceUri"].as_str()?).ok()?;
        let path = uri.to_file_path().ok()?;
        let position: lsp_types::Position =
            serde_json::from_value(params["params"]["range"]["start"].clone()).ok()?;
        let text = self.vfs.snapshot().read(&path).ok()?;
        Some((path, Text::new(&text, self.unit).offset(position)))
    }

    /// `elm/getMoveDestinations`: the project's other modules.
    pub(crate) fn move_destinations(
        &mut self,
        params: &Value,
    ) -> Result<Value, (ErrorCode, String)> {
        let snapshot = self.vfs.snapshot();
        let Some((path, _)) = self.move_source(params) else {
            return Ok(serde_json::json!({ "destinations": [] }));
        };
        let root =
            index::manifest(&snapshot, &path).and_then(|m| m.parent().map(Path::to_path_buf));
        let (Some(root), Some(index)) = (root, self.index(&snapshot, &path)) else {
            return Ok(serde_json::json!({ "destinations": [] }));
        };
        let destinations: Vec<Value> = (nomi_ide::move_destinations(index, &root, &path)
            .into_iter())
        .filter_map(|d| {
            let uri = Url::from_file_path(&d.path).ok()?;
            Some(serde_json::json!({ "name": d.name, "path": d.folder, "uri": uri }))
        })
        .collect();
        Ok(serde_json::json!({ "destinations": destinations }))
    }

    /// `elm/move`: the function moved, applied by the client as one
    /// refactoring.
    pub(crate) fn move_to(&mut self, params: &Value) -> Result<Value, (ErrorCode, String)> {
        let snapshot = self.vfs.snapshot();
        let destination = (params["destination"]["uri"].as_str())
            .and_then(|u| Url::parse(u).ok())
            .and_then(|u| u.to_file_path().ok());
        let (Some((path, offset)), Some(destination)) = (self.move_source(params), destination)
        else {
            return Ok(Value::Null);
        };
        let unit = self.unit;
        let Some(index) = self.index(&snapshot, &path) else {
            return Ok(Value::Null);
        };
        let Some(files) = nomi_ide::move_function(&snapshot, index, &path, offset, &destination)
        else {
            return Ok(Value::Null);
        };
        let mut changes = serde_json::Map::new();
        for (file, edits) in files {
            let (Ok(uri), Ok(text)) = (Url::from_file_path(&file), snapshot.read(&file)) else {
                continue;
            };
            let text = Text::new(&text, unit);
            let edits: Vec<TextEdit> = (edits.into_iter())
                .map(|edit| match edit {
                    nomi_ide::MoveEdit::Edit(e) => TextEdit::new(text.range(e.range), e.text),
                    nomi_ide::MoveEdit::AtLine(line, inserted) => {
                        let at = lsp_types::Position::new(line, 0);
                        TextEdit::new(lsp_types::Range::new(at, at), inserted)
                    }
                })
                .collect();
            changes.insert(uri.to_string(), serde_json::json!(edits));
        }
        let id = lsp_server::RequestId::from("nomi/move".to_owned());
        let params = serde_json::json!({
            "label": "Move Function",
            "edit": { "changes": changes },
            "metadata": { "isRefactoring": true },
        });
        send(
            &self.sender,
            Message::Request(Request::new(id, "workspace/applyEdit".to_owned(), params)),
        );
        Ok(Value::Null)
    }

    pub(crate) fn show(&self, kind: lsp_types::MessageType, message: &str) {
        let params = lsp_types::ShowMessageParams {
            typ: kind,
            message: message.to_owned(),
        };
        send(
            &self.sender,
            notification::<lsp_types::notification::ShowMessage>(params),
        );
    }

    pub(crate) fn code_actions(
        &mut self,
        params: &lsp_types::CodeActionParams,
    ) -> Result<Value, (ErrorCode, String)> {
        let uri = &params.text_document.uri;
        let document = self.document(uri)?;
        let (path, text) = (document.path.clone(), document.text.clone());
        let text = Text::new(&text, self.unit);
        let given_json = &params.context.diagnostics;
        let given: Vec<nomi_ide::Diagnosed> = (given_json.iter())
            .map(|d| nomi_ide::Diagnosed {
                code: (d.data.as_ref())
                    .and_then(|data| data["code"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
                message: d.message.clone(),
                range: text.text_range(d.range),
                severity: match d.severity {
                    Some(DiagnosticSeverity::ERROR) => nomi_ide::Severity::Error,
                    Some(DiagnosticSeverity::INFORMATION | DiagnosticSeverity::HINT) => {
                        nomi_ide::Severity::Information
                    }
                    _ => nomi_ide::Severity::Warning,
                },
                source: d.source.clone().unwrap_or_default(),
                unnecessary: (d.tags.iter().flatten()).any(|t| *t == DiagnosticTag::UNNECESSARY),
                // elm-review's fixes, as its diagnostic carries them.
                fixes: (d.data.as_ref())
                    .and_then(|data| data["fixes"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|fix| {
                        let range: lsp_types::Range =
                            serde_json::from_value(fix["range"].clone()).ok()?;
                        Some(nomi_ide::Edit {
                            range: text.text_range(range),
                            text: fix["string"].as_str()?.to_owned(),
                        })
                    })
                    .collect(),
            })
            .collect();
        let problems = (self.latest.lock().unwrap().get(&path).cloned()).unwrap_or_default();
        let snapshot = self.vfs.snapshot();
        if self.index(&snapshot, &path).is_none() {
            return Ok(Value::Array(Vec::new()));
        }
        let index = &self.index.as_ref().expect("indexed above").2;
        let store = &self.store;
        let packages = self
            .packages
            .get_or_insert_with(|| nomi_ide::PackageModules::read(store));
        let request = nomi_ide::Request {
            range: text.text_range(params.range),
            given: &given,
            problems: &problems,
            packages,
            lazy: self.resolves_edits,
            move_function: self.settings.lock().unwrap().move_function,
        };
        let actions = nomi_ide::code_actions(&snapshot, index, &path, &request);
        let mut found = Vec::new();
        for action in actions {
            let action_command = action.command.clone();
            let mut json = serde_json::Map::new();
            json.insert("title".into(), action.title.into());
            let kind = match action.kind {
                nomi_ide::ActionKind::QuickFix => "quickfix",
                nomi_ide::ActionKind::Refactor => "refactor",
                nomi_ide::ActionKind::RefactorExtract => "refactor.extract",
                nomi_ide::ActionKind::RefactorMove => "refactor.move",
            };
            json.insert("kind".into(), kind.into());
            match action.diagnostics {
                Some(nomi_ide::ActionDiagnostics::Given(i)) => {
                    json.insert("diagnostics".into(), serde_json::json!([given_json[i]]));
                }
                Some(nomi_ide::ActionDiagnostics::Program(diagnostics)) => {
                    let diagnostics: Vec<Value> = (diagnostics.iter())
                        .map(|d| {
                            let mut json = serde_json::json!({
                                "code": d.code, "message": d.message,
                                "range": text.range(d.range),
                                "severity": severity_number(d.severity),
                                "source": d.source, "uri": uri,
                            });
                            if d.unnecessary {
                                json["tags"] = serde_json::json!([1]);
                            }
                            json
                        })
                        .collect();
                    json.insert("diagnostics".into(), diagnostics.into());
                }
                None => {}
            }
            if let Some(files) = action.edits {
                let mut changes = serde_json::Map::new();
                for (file, edits) in files {
                    let (Ok(file_uri), Ok(file_text)) =
                        (Url::from_file_path(&file), snapshot.read(&file))
                    else {
                        continue;
                    };
                    let file_text = Text::new(&file_text, self.unit);
                    let edits: Vec<TextEdit> = (edits.iter())
                        .map(|e| TextEdit::new(file_text.range(e.range), e.text.clone()))
                        .collect();
                    changes.insert(file_uri.to_string(), serde_json::json!(edits));
                }
                json.insert("edit".into(), serde_json::json!({ "changes": changes }));
            }
            match action.data {
                nomi_ide::ActionData::Fix { id, all: false } => {
                    json.insert("data".into(), serde_json::json!({ "fixId": id }));
                }
                nomi_ide::ActionData::Fix { id, all: true } => {
                    json.insert(
                        "data".into(),
                        serde_json::json!({ "fixId": id, "isFixAll": true }),
                    );
                }
                nomi_ide::ActionData::Refactor {
                    name,
                    action,
                    range,
                    rename,
                } => {
                    let mut data = serde_json::json!({
                        "actionName": action, "refactorName": name,
                        "range": text.range(range), "uri": uri,
                    });
                    if let Some(rename) = rename {
                        data["renamePosition"] = serde_json::json!(rename_position(&text, rename));
                    }
                    if let Some(nomi_ide::ActionCommand::MoveFunction { name }) = &action_command {
                        let root = (index::manifest(&snapshot, &path))
                            .and_then(|m| m.parent().and_then(|r| Url::from_file_path(r).ok()))
                            .map(|r| r.as_str().trim_end_matches('/').to_owned())
                            .unwrap_or_default();
                        json.insert(
                            "command".into(),
                            serde_json::json!({
                                "title": "Refactor",
                                "command": format!("elm.refactor-{root}"),
                                "arguments": [
                                    "moveFunction",
                                    { "textDocument": { "uri": uri }, "range": text.range(range) },
                                    name,
                                ],
                            }),
                        );
                    }
                    json.insert("data".into(), data);
                }
                nomi_ide::ActionData::None => {}
            }
            if let Some(preferred) = action.preferred {
                json.insert("isPreferred".into(), preferred.into());
            }
            if let Some(nomi_ide::ActionCommand::InstallPackage { package }) = &action_command {
                json.insert(
                    "command".into(),
                    serde_json::json!({
                        "title": "Install Package",
                        "command": INSTALL_PACKAGE,
                        "arguments": [uri, package],
                    }),
                );
            }
            found.push(Value::Object(json));
        }
        Ok(Value::Array(found))
    }
}

/// Runs `publish` if `snapshot` is still the latest revision, holding
/// `publishing` so that no newer one is made meanwhile. Whether it ran.
/// The command an install action runs.
pub(crate) const INSTALL_PACKAGE: &str = "nomi.installPackage";

/// Runs `elm install package` in `root` as nomi does it, answering its
/// question with `answer`: whether it asked, and what it printed.
pub(crate) fn install_package(root: &Path, package: &str, answer: Option<&str>) -> (bool, String) {
    let Ok(cache) = Cache::locate() else {
        return (
            false,
            "nomi has no cache directory for packages.".to_owned(),
        );
    };
    let mut console = Captured {
        answer: answer.map(str::to_owned),
        asked: false,
        printed: String::new(),
    };
    nomi_engine::install::run(
        &Vfs::new(),
        &mut console,
        nomi_engine::console::Personality::Elm,
        root,
        &cache,
        &nomi_engine::Web::new(),
        Some(package),
    );
    (console.asked, console.printed)
}

/// A console that keeps what is printed and answers one question.
struct Captured {
    answer: Option<String>,
    asked: bool,
    printed: String,
}

impl nomi_engine::console::Console for Captured {
    fn out(&mut self, text: &str) {
        self.printed.push_str(text);
    }

    fn err(&mut self, text: &str) {
        self.printed.push_str(text);
    }

    fn read_line(&mut self) -> std::io::Result<Option<String>> {
        self.asked = true;
        Ok(self.answer.take())
    }

    fn read_all(&mut self) -> std::io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

/// A refactor's rename position, where the client starts renaming.
pub(crate) fn rename_position(text: &Text, rename: nomi_ide::RenameAt) -> lsp_types::Position {
    let at = text.position(rename.from);
    lsp_types::Position::new(at.line + rename.lines, at.character + rename.columns)
}

pub(crate) fn severity_number(severity: nomi_ide::Severity) -> u8 {
    match severity {
        nomi_ide::Severity::Error => 1,
        nomi_ide::Severity::Warning => 2,
        nomi_ide::Severity::Information => 3,
    }
}
