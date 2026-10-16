//! Diagnostics: the problems `nomi_ide` finds in every project with an
//! open file, published as elm-language-server publishes them. One worker
//! thread runs the checks in turn. A change waits a moment for the next
//! one, and only the latest revision is checked, a check stopping when a
//! newer revision comes; opening or saving a file is checked at once.
//! With `onlyUpdateDiagnosticsOnSave`, only opening and saving are.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use lsp_server::Message;
use lsp_types::notification::PublishDiagnostics;
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DiagnosticTag, PublishDiagnosticsParams, Range, Url,
};
use nomi_base::{ColumnUnit, lock};
use nomi_engine::artifacts::Artifacts;
use nomi_engine::index::{Index, MemberBuilds, Memo};
use nomi_engine::live::Live;
use nomi_engine::schedule::Cancelled;
use nomi_engine::{Snapshot, Store, live};
use nomi_ide::{CheckSettings, Code, Problem, Reviewed, Severity};
use serde::{Deserialize, Serialize};

use crate::indexes::Indexes;
use crate::position::Text;
use crate::settings::Settings;
use crate::{notification, publish_if_current, send};

/// How long a change waits for the next before it's checked.
const DEBOUNCE: Duration = Duration::from_millis(200);

/// Why the open documents are checked again.
pub(crate) enum Cause {
    /// A file was opened or saved, or the client asked.
    Saved,
    Closed,
    /// The editor changed the file at the path, whose elm-review
    /// problems go.
    Edited(PathBuf),
}

/// A revision to check: its snapshot, the open files, and why.
pub(crate) struct Job {
    pub(crate) snapshot: Snapshot,
    pub(crate) open: Vec<(Url, PathBuf)>,
    pub(crate) cause: Cause,
}

/// The jobs one check takes: the latest, whether a file was opened or
/// saved for any of them, and the files edited for them.
struct Batch {
    job: Job,
    saved: bool,
    changed: Vec<PathBuf>,
}

/// The worker's end of the server.
pub(crate) struct Diagnostics {
    jobs: Sender<Job>,
}

/// What the server and its diagnostics worker share.
pub(crate) struct Shared {
    /// Where navigation and checks find the packages a project uses,
    /// never downloading them.
    pub(crate) store: Store,
    pub(crate) indexes: Indexes,
    /// nomi's artifact cache, which checks reuse as `nomi make` does.
    pub(crate) artifacts: Option<Artifacts>,
    /// The packages' interfaces, made once per server.
    pub(crate) memo: Mutex<Memo>,
    /// The workspaces' member packages, as checks and indexes last built
    /// them.
    pub(crate) members: MemberBuilds,
    pub(crate) unit: ColumnUnit,
    pub(crate) sender: Sender<Message>,
    /// Held to make a revision, and to publish what a snapshot found
    /// while it is still the latest, so nothing stale is published after
    /// something newer.
    pub(crate) publishing: Mutex<()>,
    pub(crate) settings: Mutex<Settings>,
    /// The problems last published, by file, which code actions read.
    pub(crate) latest: Mutex<HashMap<PathBuf, Vec<Problem>>>,
}

impl Diagnostics {
    pub(crate) fn start(shared: Arc<Shared>) -> Diagnostics {
        let (jobs, received) = unbounded();
        thread::spawn(move || work(&shared, &received));
        Diagnostics { jobs }
    }

    pub(crate) fn check(&self, job: Job) {
        let _ = self.jobs.send(job);
    }
}

fn work(shared: &Shared, jobs: &Receiver<Job>) {
    // Files published with problems, which are cleared when they have
    // none.
    let mut published: HashSet<Url> = HashSet::new();
    let mut reviewed = Reviewed::default();
    // A save whose check a newer revision cancelled: that revision's check
    // takes it over, as if it had come while the save's waited.
    let mut cancelled_save = false;
    while let Ok(job) = jobs.recv() {
        let Some(batch) = coalesce(job, jobs, std::mem::take(&mut cancelled_save)) else {
            return;
        };
        for path in &batch.changed {
            reviewed.changed(path);
        }
        let settings = lock(&shared.settings).checks.clone();
        if settings.on_save_only && !batch.saved {
            continue;
        }
        let snapshot = &batch.job.snapshot;
        // Every open file is published, with no diagnostics where it has
        // none.
        let open_files = || -> BTreeMap<Url, Vec<Diagnostic>> {
            (batch.job.open.iter())
                .map(|(uri, _)| (uri.clone(), Vec::new()))
                .collect()
        };
        // A bug in one check, or in turning its problems into diagnostics,
        // publishes none for it rather than end diagnostics for the
        // session, or leave the client waiting.
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let found = problems(shared, &batch, &settings, &mut reviewed)?;
            let mut files = open_files();
            let by_file = diagnostics(found, snapshot, shared.unit, &mut files);
            Ok((by_file, files))
        }));
        let (by_file, mut files) = match checked {
            Ok(Ok(checked)) => checked,
            Ok(Err(Cancelled)) => {
                cancelled_save = batch.saved;
                continue;
            }
            Err(_) => (HashMap::new(), open_files()),
        };
        publish_if_current(&shared.publishing, snapshot, || {
            *lock(&shared.latest) = by_file;
            for uri in published.drain() {
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

/// `first` and the jobs that come while it waits, checked as one: after a
/// change it waits a moment for the next, and not at all once a file was
/// opened or saved for it, or `saved` says one was before. None once the
/// server is gone.
fn coalesce(first: Job, jobs: &Receiver<Job>, saved: bool) -> Option<Batch> {
    let mut batch = Batch {
        job: first,
        saved,
        changed: Vec::new(),
    };
    loop {
        match &batch.job.cause {
            Cause::Saved => batch.saved = true,
            Cause::Closed => {}
            Cause::Edited(path) => batch.changed.push(path.clone()),
        }
        let wait = if batch.saved {
            Duration::ZERO
        } else {
            DEBOUNCE
        };
        match jobs.recv_timeout(wait) {
            Ok(next) => batch.job = next,
            Err(RecvTimeoutError::Timeout) => return Some(batch),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// What `nomi_ide` finds in the batch's projects, from the packages built
/// before and kept for the checks after; cancelled once a newer revision
/// makes the batch's stale.
fn problems(
    shared: &Shared,
    batch: &Batch,
    settings: &CheckSettings,
    reviewed: &mut Reviewed,
) -> Result<Vec<Problem>, Cancelled> {
    let snapshot = &batch.job.snapshot;
    let open: Vec<PathBuf> = (batch.job.open.iter())
        .map(|(_, path)| path.clone())
        .collect();
    let mut memo = lock(&shared.memo).clone();
    let found = nomi_ide::project_problems(
        snapshot,
        &open,
        settings,
        batch.saved,
        reviewed,
        shared.unit,
        |manifest, open, index_from| check(shared, snapshot, &mut memo, manifest, open, index_from),
    );
    lock(&shared.memo).extend(memo);
    found
}

/// The project at `manifest` checked as `nomi make` checks it, with the
/// `open` files, and indexed meanwhile from `index_from` where given, the
/// packages either builds kept in `memo`.
fn check(
    shared: &Shared,
    snapshot: &Snapshot,
    memo: &mut Memo,
    manifest: &Path,
    open: &[PathBuf],
    index_from: Option<&Path>,
) -> Result<(Option<Live>, Option<Arc<Index>>), Cancelled> {
    thread::scope(|scope| {
        let indexing = index_from.map(|first| {
            let mut memo = memo.clone();
            scope.spawn(move || {
                let kept = &shared.members;
                let index =
                    (shared.indexes).get(snapshot, &shared.store, &mut memo, kept, manifest, first);
                (index, memo)
            })
        });
        let artifacts = shared.artifacts.as_ref();
        let kept = &shared.members;
        let checked = live::check(
            snapshot,
            &shared.store,
            manifest,
            open,
            memo,
            kept,
            artifacts,
        );
        let index = (indexing.and_then(|thread| thread.join().ok())).and_then(|(index, built)| {
            memo.extend(built);
            index
        });
        checked.map(|live| (live, index))
    })
}

/// `found` by file, each file's diagnostics added to `files`, its text
/// read once for their positions.
fn diagnostics(
    found: Vec<Problem>,
    snapshot: &Snapshot,
    unit: ColumnUnit,
    files: &mut BTreeMap<Url, Vec<Diagnostic>>,
) -> HashMap<PathBuf, Vec<Problem>> {
    let mut by_file: HashMap<PathBuf, Vec<Problem>> = HashMap::new();
    for problem in found {
        by_file
            .entry(problem.path.clone())
            .or_default()
            .push(problem);
    }
    for (path, problems) in &by_file {
        let (Ok(uri), Ok(text)) = (Url::from_file_path(path), snapshot.read(path)) else {
            continue;
        };
        let text = Text::new(text, unit);
        let diagnostics: Vec<Diagnostic> = problems
            .iter()
            .map(|p| diagnostic(p, &uri, &text))
            .collect();
        files.entry(uri).or_default().extend(diagnostics);
    }
    by_file
}

/// What a diagnostic carries for the code actions it's sent back with, as
/// elm-language-server writes it.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct DiagnosticData {
    pub(crate) uri: String,
    pub(crate) code: String,
    /// elm-review's fixes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fixes: Option<Vec<ReviewFix>>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct ReviewFix {
    pub(crate) string: String,
    pub(crate) range: Range,
}

fn diagnostic(problem: &Problem, uri: &Url, text: &Text) -> Diagnostic {
    let data = DiagnosticData {
        uri: problem.relative.clone().unwrap_or_else(|| uri.to_string()),
        code: problem.code.as_str().to_owned(),
        fixes: (problem.code == Code::ElmReview).then(|| {
            (problem.fixes.iter())
                .map(|f| ReviewFix {
                    string: f.text.clone(),
                    range: text.range(f.range),
                })
                .collect()
        }),
    };
    let data = serde_json::to_value(data).expect("a diagnostic's data serializes");
    Diagnostic {
        range: text.range(problem.range),
        severity: Some(severity(problem.severity)),
        source: Some(problem.code.source().as_str().to_owned()),
        message: problem.message.clone(),
        tags: (problem.unnecessary).then(|| vec![DiagnosticTag::UNNECESSARY]),
        data: Some(data),
        ..Diagnostic::default()
    }
}

/// `severity` as the protocol numbers it.
pub(crate) fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Information => DiagnosticSeverity::INFORMATION,
    }
}
