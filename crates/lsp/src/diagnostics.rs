//! Diagnostics (plan 9.3): every project with an open file checked as
//! `elm make` checks it, and published as elm-language-server publishes
//! its `elm make` diagnostics, with the annotations an open file leaves
//! out. One worker thread runs the checks in turn. A change waits a
//! moment for the next one, and only the latest revision is checked;
//! opening or saving a file is checked at once. With
//! `onlyUpdateDiagnosticsOnSave`, only opening and saving are, and
//! annotations aren't suggested, as elm-language-server does then.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use lsp_server::Message;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::{Diagnostic, DiagnosticSeverity, PublishDiagnosticsParams, Url};
use nomi_base::ColumnUnit;
use nomi_engine::index::{self, Memo};
use nomi_engine::{Snapshot, Store, live};
use nomi_ide::{Problem, Severity};
use serde_json::{Value, json};

use crate::position::Text;
use crate::{notification, publish_if_current, send};

/// How long a change waits for the next before it's checked.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// The client's settings that diagnostics heed, from its `elmLS` section.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Settings {
    pub on_save_only: bool,
}

impl Settings {
    /// Reads `value`: the `elmLS` section, or settings that hold it.
    pub fn read(value: &Value) -> Option<Settings> {
        let section = value.get("elmLS").unwrap_or(value);
        let section = section.as_object()?;
        Some(Settings {
            on_save_only: (section.get("onlyUpdateDiagnosticsOnSave"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// A revision to check: its snapshot, the open files, and whether a
/// file was opened or saved for it.
pub(crate) struct Job {
    pub snapshot: Snapshot,
    pub open: Vec<(Url, PathBuf)>,
    pub saved: bool,
}

/// The worker's end of the server.
pub(crate) struct Diagnostics {
    jobs: Sender<Job>,
}

/// What the worker shares with the server.
pub(crate) struct Shared {
    pub store: Store,
    pub memo: Arc<Mutex<Memo>>,
    pub unit: ColumnUnit,
    pub sender: Sender<Message>,
    pub publishing: Arc<Mutex<()>>,
    pub settings: Arc<Mutex<Settings>>,
}

impl Diagnostics {
    pub fn start(shared: Shared) -> Diagnostics {
        let (jobs, received) = unbounded();
        thread::spawn(move || work(&shared, &received));
        Diagnostics { jobs }
    }

    pub fn check(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

fn work(shared: &Shared, jobs: &Receiver<Job>) {
    // Files published with problems, which are cleared when they have
    // none.
    let mut published: HashSet<Url> = HashSet::new();
    while let Ok(mut job) = jobs.recv() {
        let mut saved = job.saved;
        loop {
            let wait = if saved { Duration::ZERO } else { DEBOUNCE };
            match jobs.recv_timeout(wait) {
                Ok(next) => {
                    saved |= next.saved;
                    job = next;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let settings = shared.settings.lock().unwrap().clone();
        if settings.on_save_only && !saved {
            continue;
        }
        let found = problems(shared, &job, !settings.on_save_only);
        let mut files: BTreeMap<Url, Vec<Diagnostic>> = BTreeMap::new();
        for (uri, _) in &job.open {
            files.entry(uri.clone()).or_default();
        }
        for problem in found {
            let Ok(uri) = Url::from_file_path(&problem.path) else {
                continue;
            };
            let Ok(text) = job.snapshot.read(&problem.path) else {
                continue;
            };
            let diagnostic = diagnostic(&problem, &uri, &Text::new(&text, shared.unit));
            files.entry(uri).or_default().push(diagnostic);
        }
        publish_if_current(&shared.publishing, &job.snapshot, || {
            for uri in published.drain().collect::<Vec<_>>() {
                files.entry(uri).or_default();
            }
            for (uri, diagnostics) in files {
                if !diagnostics.is_empty() {
                    published.insert(uri.clone());
                }
                let params = PublishDiagnosticsParams::new(uri, diagnostics, None);
                send(&shared.sender, notification::<PublishDiagnostics>(params));
            }
        });
    }
}

/// The problems of every project with an open file: elm's, and the
/// annotations the open files leave out where `annotations`.
fn problems(shared: &Shared, job: &Job, annotations: bool) -> Vec<Problem> {
    let snapshot = &job.snapshot;
    let mut projects: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut found = Vec::new();
    for (_, path) in &job.open {
        match index::manifest(snapshot, path) {
            Some(manifest) => projects.entry(manifest).or_default().push(path.clone()),
            None => found.extend(syntax(snapshot, path, None)),
        }
    }
    // The packages are built once and kept; each check starts from what
    // was built before it.
    let mut memo = shared.memo.lock().unwrap().clone();
    for (manifest, open) in projects {
        match live::check(snapshot, &shared.store, &manifest, &open, &mut memo) {
            Some(live) => found.extend(nomi_ide::compiler_problems(snapshot, &live, &open)),
            None => {
                for path in &open {
                    found.extend(syntax(snapshot, path, manifest.parent()));
                }
            }
        }
        if annotations
            && let Some(index) = index::index(snapshot, &shared.store, &open[0], &mut memo)
        {
            for path in &open {
                found.extend(nomi_ide::missing_annotations(&index, path));
            }
        }
    }
    shared.memo.lock().unwrap().extend(memo);
    found
}

/// The syntax error in the file at `path`, for a file nomi can't check
/// with its project: one outside any, or whose dependencies the store
/// doesn't hold.
fn syntax(snapshot: &Snapshot, path: &Path, root: Option<&Path>) -> Vec<Problem> {
    let found = nomi_ide::diagnostics(snapshot, path).unwrap_or_default();
    let relative = match root.and_then(|root| path.strip_prefix(root).ok()) {
        Some(relative) => relative,
        None => Path::new(path.file_name().unwrap_or_default()),
    };
    let relative: Vec<_> = relative.iter().map(|p| p.to_string_lossy()).collect();
    (found.into_iter())
        .map(|d| Problem {
            path: path.to_owned(),
            range: d.range,
            severity: Severity::Error,
            message: format!("{} - {}", d.title, d.message),
            code: "elm_make",
            relative: Some(relative.join("/")),
        })
        .collect()
}

fn diagnostic(problem: &Problem, uri: &Url, text: &Text) -> Diagnostic {
    let data_uri = problem.relative.clone().unwrap_or_else(|| uri.to_string());
    Diagnostic {
        range: text.range(problem.range),
        severity: Some(match problem.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Information => DiagnosticSeverity::INFORMATION,
        }),
        source: Some("Elm".to_owned()),
        message: problem.message.clone(),
        data: Some(json!({ "uri": data_uri, "code": problem.code })),
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_from_the_section_or_around_it() {
        let on = Settings { on_save_only: true };
        let section = json!({ "onlyUpdateDiagnosticsOnSave": true });
        assert_eq!(Settings::read(&section), Some(on.clone()));
        assert_eq!(Settings::read(&json!({ "elmLS": section })), Some(on));
        assert_eq!(Settings::read(&json!({})), Some(Settings::default()));
        assert_eq!(Settings::read(&Value::Null), None);
    }
}
