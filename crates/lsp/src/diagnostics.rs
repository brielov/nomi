//! Diagnostics (plan 9.3): every project with an open file checked as
//! `elm make` checks it, and published as elm-language-server publishes
//! its `elm make` diagnostics, with the annotations an open file leaves
//! out. One worker thread runs the checks in turn. A change waits a
//! moment for the next one, and only the latest revision is checked;
//! opening or saving a file is checked at once. With
//! `onlyUpdateDiagnosticsOnSave`, only opening and saving are, and
//! annotations aren't suggested, as elm-language-server does then.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use lsp_server::Message;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, PublishDiagnosticsParams, Url};
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
    pub no_lint: bool,
    /// Whether the client runs `elm.expose-` and `elm.unexpose-`
    /// commands, which lenses then offer.
    pub expose_commands: bool,
    /// Whether a package is installed without asking first.
    pub skip_install_confirmation: bool,
    /// Whether the client asks where to move a function, and moves it.
    pub move_function: bool,
    /// How elm-review's problems show: not at all, as warnings or errors.
    pub review: Review,
    /// The elm-review, elm and elm-format the client names, if it does.
    pub review_path: String,
    pub elm_path: String,
    pub elm_format_path: String,
}

/// `elmReviewDiagnostics`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Review {
    #[default]
    Off,
    Warning,
    Error,
}

impl Settings {
    /// Reads `value`: the `elmLS` section, or settings that hold it.
    pub fn read(value: &Value) -> Option<Settings> {
        let section = value.get("elmLS").unwrap_or(value);
        let section = section.as_object()?;
        let flag = |name: &str| section.get(name).and_then(Value::as_bool).unwrap_or(false);
        let text = |name: &str| {
            (section.get(name).and_then(Value::as_str))
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        Some(Settings {
            on_save_only: flag("onlyUpdateDiagnosticsOnSave"),
            no_lint: flag("disableElmLSDiagnostics"),
            skip_install_confirmation: flag("skipInstallPackageConfirmation"),
            review: match section.get("elmReviewDiagnostics").and_then(Value::as_str) {
                Some("warning") => Review::Warning,
                Some("error") => Review::Error,
                _ => Review::Off,
            },
            review_path: text("elmReviewPath"),
            elm_path: text("elmPath"),
            elm_format_path: text("elmFormatPath"),
            move_function: (section.get("extendedCapabilities"))
                .and_then(|e| e.get("moveFunctionRefactoringSupport"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            expose_commands: (section.get("extendedCapabilities"))
                .and_then(|e| e.get("exposeUnexposeSupport"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Whether the lint rules run. Diagnostics on save run them on open
    /// and save whatever `disableElmLSDiagnostics` says, as
    /// elm-language-server does.
    fn lint(&self) -> bool {
        self.on_save_only || !self.no_lint
    }
}

/// A project's `elm-analyse.json`: which lint rules run, and which files
/// have none.
struct Analyse {
    checks: nomi_ide::Checks,
    excluded: Vec<PathBuf>,
}

impl Analyse {
    fn read(snapshot: &Snapshot, root: &Path) -> Analyse {
        let json = (snapshot.read(&root.join("elm-analyse.json")).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        let checks = (json
            .get("checks")
            .and_then(Value::as_object)
            .into_iter()
            .flatten())
        .filter_map(|(name, on)| Some((name.clone(), on.as_bool()?)))
        .collect::<Vec<_>>();
        let excluded = (json
            .get("excludedPaths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten())
        .filter_map(Value::as_str)
        .map(|path| root.join(path))
        .collect();
        Analyse {
            checks: nomi_ide::Checks::new(checks),
            excluded,
        }
    }

    fn excludes(&self, path: &Path) -> bool {
        self.excluded.iter().any(|dir| path.starts_with(dir))
    }
}

/// A revision to check: its snapshot, the open files, and whether a
/// file was opened or saved for it.
pub(crate) struct Job {
    pub snapshot: Snapshot,
    pub open: Vec<(Url, PathBuf)>,
    pub saved: bool,
    /// The files the editor changed for it, whose elm-review problems go.
    pub changed: Vec<PathBuf>,
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
    /// The problems last published, by file, which code actions read.
    pub latest: Arc<Mutex<HashMap<PathBuf, Vec<Problem>>>>,
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
    // elm-review's problems by file, from the last open or save that ran
    // it, less the files changed since.
    let mut reviewed: HashMap<PathBuf, Vec<Problem>> = HashMap::new();
    while let Ok(mut job) = jobs.recv() {
        let mut saved = job.saved;
        let mut changed = std::mem::take(&mut job.changed);
        loop {
            let wait = if saved { Duration::ZERO } else { DEBOUNCE };
            match jobs.recv_timeout(wait) {
                Ok(mut next) => {
                    saved |= next.saved;
                    changed.append(&mut next.changed);
                    job = next;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        for path in &changed {
            reviewed.remove(path);
        }
        let settings = shared
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if settings.on_save_only && !saved {
            continue;
        }
        // A bug in one check publishes nothing for it rather than end
        // diagnostics for the session.
        let Ok(found) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            problems(shared, &job, &settings, saved, &mut reviewed)
        })) else {
            continue;
        };
        let mut by_file: HashMap<PathBuf, Vec<Problem>> = HashMap::new();
        for problem in &found {
            by_file
                .entry(problem.path.clone())
                .or_default()
                .push(problem.clone());
        }
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
            *shared
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = by_file;
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

/// The problems of every project with an open file: elm's, the lint
/// rules' in open files, and the annotations they leave out, as
/// `settings` has them.
fn problems(
    shared: &Shared,
    job: &Job,
    settings: &Settings,
    saved: bool,
    reviewed: &mut HashMap<PathBuf, Vec<Problem>>,
) -> Vec<Problem> {
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
    let mut memo = shared
        .memo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    for (manifest, open) in projects {
        let root = manifest.parent().unwrap_or(Path::new(""));
        let before = found.len();
        match live::check(snapshot, &shared.store, &manifest, &open, &mut memo) {
            Some(live) => found.extend(nomi_ide::compiler_problems(snapshot, &live, &open)),
            None => {
                for path in &open {
                    found.extend(syntax(snapshot, path, Some(root)));
                }
            }
        }
        // elm-review, on open and save, once elm finds nothing; its
        // problems go when elm finds some.
        let clean = found.len() == before;
        if saved || !clean {
            reviewed.retain(|path, _| !path.starts_with(root));
        }
        if saved && clean {
            for problem in review(snapshot, root, settings, shared.unit) {
                reviewed
                    .entry(problem.path.clone())
                    .or_default()
                    .push(problem);
            }
        }
        let mut review_found: Vec<&Problem> = (reviewed.iter())
            .filter(|(path, _)| path.starts_with(root))
            .flat_map(|(_, problems)| problems)
            .collect();
        review_found.sort_by(|a, b| (&a.path, a.range.start()).cmp(&(&b.path, b.range.start())));
        found.extend(review_found.into_iter().cloned());
        let analyse = Analyse::read(snapshot, root);
        let open: Vec<&PathBuf> = open.iter().filter(|p| !analyse.excludes(p)).collect();
        let annotations = !settings.on_save_only && analyse.checks.on("MissingTypeAnnotation");
        // Indexing takes a while; only annotations and ports need it.
        let ports = || {
            open.iter()
                .any(|p| snapshot.read(p).is_ok_and(|t| t.contains("port ")))
        };
        let index = match open.first() {
            Some(first) if annotations || (settings.lint() && ports()) => {
                index::index(snapshot, &shared.store, first, &mut memo)
            }
            _ => None,
        };
        if settings.lint() {
            for path in &open {
                found.extend(nomi_ide::lint(
                    snapshot,
                    index.as_ref(),
                    path,
                    &analyse.checks,
                ));
            }
        }
        if annotations && let Some(index) = &index {
            for path in &open {
                found.extend(nomi_ide::missing_annotations(index, path));
            }
        }
    }
    shared
        .memo
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .extend(memo);
    found
}

/// elm-review's problems in the project at `root`, where the client
/// turned it on and the project configures it: its CLI's JSON report,
/// as elm-language-server runs it.
fn review(snapshot: &Snapshot, root: &Path, settings: &Settings, unit: ColumnUnit) -> Vec<Problem> {
    if settings.review == Review::Off || !root.join("review/src/ReviewConfig.elm").is_file() {
        return Vec::new();
    }
    let program = match settings.review_path.as_str() {
        "" => PathBuf::from("elm-review"),
        // A path the client gives relative is the project's.
        path if path.contains(['/', '\\']) => root.join(path),
        path => PathBuf::from(path),
    };
    let mut command = if cfg!(windows) && program.extension().is_some_and(|e| e == "js") {
        // Windows runs no script by its first line.
        let mut node = std::process::Command::new("node");
        node.arg(&program);
        node
    } else {
        std::process::Command::new(&program)
    };
    command.args(["--report", "json", "--namespace", "vscode"]);
    if !settings.elm_path.is_empty() {
        command.args(["--compiler", &settings.elm_path]);
    }
    if !settings.elm_format_path.is_empty() {
        command.args(["--elm-format-path", &settings.elm_format_path]);
    }
    let Ok(output) = command
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if output.status.success() {
        return Vec::new();
    }
    let Ok(report) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Vec::new();
    };
    if report["type"] != "review-errors" {
        return Vec::new();
    }
    let severity = match settings.review {
        Review::Error => Severity::Error,
        _ => Severity::Warning,
    };
    let mut found = Vec::new();
    for file in report["errors"].as_array().into_iter().flatten() {
        let path = root.join(file["path"].as_str().unwrap_or_default());
        let Ok(text) = snapshot.read(&path) else {
            continue;
        };
        let text = Text::new(&text, unit);
        let range = |region: &Value| {
            let at = |p: &Value| {
                let position = lsp_types::Position::new(
                    (p["line"].as_u64().unwrap_or(1) as u32).saturating_sub(1),
                    (p["column"].as_u64().unwrap_or(1) as u32).saturating_sub(1),
                );
                text.offset(position)
            };
            let (start, end) = (at(&region["start"]), at(&region["end"]));
            nomi_base::TextRange::new(start, end.max(start))
        };
        for error in file["errors"].as_array().into_iter().flatten() {
            if error["suppressed"].as_bool() == Some(true) {
                continue;
            }
            let fixes = (error["fix"].as_array().into_iter().flatten())
                .map(|fix| nomi_ide::Edit {
                    range: range(&fix["range"]),
                    text: fix["string"].as_str().unwrap_or_default().to_owned(),
                })
                .collect();
            found.push(Problem {
                path: path.clone(),
                range: range(&error["region"]),
                severity,
                message: error["message"].as_str().unwrap_or_default().to_owned(),
                source: "elm-review",
                code: "elm_review",
                unnecessary: error["rule"]
                    .as_str()
                    .is_some_and(|r| r.starts_with("NoUnused")),
                relative: None,
                fixes,
            });
        }
    }
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
            source: "Elm",
            code: "elm_make",
            unnecessary: false,
            relative: Some(relative.join("/")),
            fixes: Vec::new(),
        })
        .collect()
}

fn diagnostic(problem: &Problem, uri: &Url, text: &Text) -> Diagnostic {
    let data_uri = problem.relative.clone().unwrap_or_else(|| uri.to_string());
    Diagnostic {
        range: text.range(problem.range),
        severity: Some(match problem.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Information => DiagnosticSeverity::INFORMATION,
        }),
        source: Some(problem.source.to_owned()),
        message: problem.message.clone(),
        tags: (problem.unnecessary).then(|| vec![DiagnosticTag::UNNECESSARY]),
        data: Some(if problem.code == "elm_review" {
            let fixes: Vec<Value> = (problem.fixes.iter())
                .map(|f| json!({ "string": f.text, "range": text.range(f.range) }))
                .collect();
            json!({ "uri": data_uri, "code": problem.code, "fixes": fixes })
        } else {
            json!({ "uri": data_uri, "code": problem.code })
        }),
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elm_analyse_json_turns_rules_off_and_files_out() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{ "checks": { "UnusedImport": false, "SingleFieldRecord": true },
                        "excludedPaths": ["src/Generated"] }"#;
        std::fs::write(dir.path().join("elm-analyse.json"), json).unwrap();
        let analyse = Analyse::read(&nomi_engine::Vfs::new().snapshot(), dir.path());
        assert!(!analyse.checks.on("UnusedImport"));
        assert!(analyse.checks.on("UnusedPatternVariable"));
        assert!(analyse.checks.opted_in("SingleFieldRecord"));
        assert!(analyse.excludes(&dir.path().join("src/Generated/Api.elm")));
        assert!(!analyse.excludes(&dir.path().join("src/Main.elm")));
    }

    #[test]
    fn settings_read_from_the_section_or_around_it() {
        let on = Settings {
            on_save_only: true,
            no_lint: false,
            expose_commands: false,
            ..Settings::default()
        };
        let section = json!({ "onlyUpdateDiagnosticsOnSave": true });
        assert_eq!(Settings::read(&section), Some(on.clone()));
        assert_eq!(Settings::read(&json!({ "elmLS": section })), Some(on));
        assert_eq!(Settings::read(&json!({})), Some(Settings::default()));
        assert_eq!(Settings::read(&Value::Null), None);
    }
}
