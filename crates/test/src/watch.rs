//! `elm-test --watch` (`RunTests.runTests` with `watch`): a
//! run, then another each time the tests change. elm-test watches with
//! chokidar; nomi polls the watched files' sizes and modification times,
//! which for a test suite's few files costs little and needs no
//! dependency.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use nomi_engine::console::Console;

use crate::command::Progress;
use crate::files::{IGNORED, relative};
use crate::supervise::chalk;

/// How often the files are looked at, and how long elm-test waits for
/// changes made together.
const POLL: Duration = Duration::from_millis(100);

/// What happened to a path, as chokidar reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Event {
    Added,
    Changed,
    Removed,
}

impl Event {
    fn as_str(self) -> &'static str {
        match self {
            Event::Added => "added",
            Event::Changed => "changed",
            Event::Removed => "removed",
        }
    }
}

/// A change, as chokidar reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    event: Event,
    /// `path.relative(projectRootDir, path)`.
    path: String,
}

/// What a path was when last looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// The watched paths when last looked at.
struct Snapshot {
    root: PathBuf,
    /// `elm.json` and `tests/` are watched whatever the project says.
    elm_json: PathBuf,
    tests: PathBuf,
    /// The test source directories.
    dirs: Vec<PathBuf>,
    /// What is walked: the watched paths, each once and none another's
    /// walk sees.
    roots: Vec<PathBuf>,
    seen: BTreeMap<PathBuf, Seen>,
}

impl Snapshot {
    fn new(root: &Path) -> Snapshot {
        let (elm_json, tests) = (root.join("elm.json"), root.join("tests"));
        let mut snapshot = Snapshot {
            root: root.to_owned(),
            roots: roots([&elm_json, &tests]),
            elm_json,
            tests,
            dirs: Vec::new(),
            seen: BTreeMap::new(),
        };
        snapshot.seen = snapshot.look();
        snapshot
    }

    /// Watches `dirs` from now on, as `watcher.add` does: what they hold
    /// already isn't a change.
    fn watch(&mut self, dirs: Vec<PathBuf>) {
        if dirs != self.dirs {
            self.roots = roots([&self.elm_json, &self.tests].into_iter().chain(&dirs));
            self.dirs = dirs;
            let now = self.look();
            for (path, seen) in now {
                self.seen.entry(path).or_insert(seen);
            }
        }
    }

    /// Every watched path now.
    fn look(&self) -> BTreeMap<PathBuf, Seen> {
        let mut seen = BTreeMap::new();
        for path in &self.roots {
            walk(path, &mut seen);
        }
        seen
    }

    /// The changes since last time that start a run.
    fn changes(&mut self) -> Vec<Change> {
        let now = self.look();
        let changes = self.diff(&now);
        self.seen = now;
        changes
    }

    /// What changed from what was seen to `now` that starts a run: an
    /// `.elm` file, `elm.json`, or `tests/` appearing.
    fn diff(&self, now: &BTreeMap<PathBuf, Seen>) -> Vec<Change> {
        let mut changes = Vec::new();
        for (path, seen) in now {
            let event = match self.seen.get(path) {
                None => Event::Added,
                Some(was) if !seen.dir && !was.dir && was != seen => Event::Changed,
                Some(_) => continue,
            };
            // Of directories, only `tests/` appearing counts (`addDir`).
            if seen.dir && event == Event::Added && *path != self.tests {
                continue;
            }
            if seen.dir || self.counts(path) {
                changes.push(self.change(event, path));
            }
        }
        for (path, was) in &self.seen {
            if !was.dir && !now.contains_key(path) && self.counts(path) {
                changes.push(self.change(Event::Removed, path));
            }
        }
        changes
    }

    fn counts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "elm")
            || *path == self.elm_json
            || *path == self.tests
    }

    fn change(&self, event: Event, path: &Path) -> Change {
        Change {
            event,
            path: relative(path, &self.root).display().to_string(),
        }
    }
}

/// `paths`, each once, without those another's walk sees.
fn roots<'p>(paths: impl IntoIterator<Item = &'p PathBuf>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !roots.iter().any(|root| sees(root, path)) {
            roots.retain(|root| !sees(path, root));
            roots.push(path.clone());
        }
    }
    roots
}

/// Whether walking `root` sees `path`: it is `root`, or below it past no
/// ignored name.
fn sees(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rest| {
        (rest.components()).all(|part| !IGNORED.iter().any(|i| part.as_os_str() == *i))
    })
}

/// `path` and, for a directory, everything below it but the ignored
/// directories.
fn walk(path: &Path, seen: &mut BTreeMap<PathBuf, Seen>) {
    let Ok(meta) = fs::metadata(path) else { return };
    let dir = meta.is_dir();
    seen.insert(
        path.to_owned(),
        Seen {
            dir,
            size: meta.len(),
            modified: meta.modified().ok(),
        },
    );
    if !dir {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let ignored = IGNORED.iter().any(|i| entry.file_name() == *i);
        if !ignored {
            walk(&entry.path(), seen);
        }
    }
}

/// `watcherEventMessage`.
fn message(queue: &[Change]) -> String {
    let mut paths: Vec<&str> = Vec::new();
    for change in queue {
        if !paths.contains(&change.path.as_str()) {
            paths.push(&change.path);
        }
    }
    if let [_] = paths.as_slice() {
        return format!("{} {}", queue[0].path, queue[0].event.as_str());
    }
    let mut events: Vec<Event> = queue.iter().map(|c| c.event).collect();
    events.sort_unstable();
    events.dedup();
    let events: Vec<&str> = events.into_iter().map(Event::as_str).collect();
    format!("{} files {}", paths.len(), events.join("/"))
}

/// One run: the test source directories it found.
pub(crate) type Once<'a> = dyn FnMut(&mut dyn Console, &mut Progress) -> Vec<PathBuf> + 'a;

/// Runs the tests with `once` each time they change, forever: once gives
/// the test source directories to watch.
pub(crate) fn watch(
    console: &mut dyn Console,
    progress: &mut Progress,
    root: &Path,
    color: bool,
    once: &mut Once<'_>,
) -> i32 {
    let blue = |text: &str| chalk(color, 34, text);
    let mut snapshot = Snapshot::new(root);
    progress.clear_console(console);
    let mut queue: Vec<Change> = Vec::new();
    loop {
        // Changes made together join the run they start.
        if !queue.is_empty() {
            let before = queue.len();
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
            if queue.len() > before {
                progress.clear_line(console);
                progress.log(console, &message(&queue));
            }
        }
        queue.clear();
        let dirs = once(console, progress);
        queue.extend(snapshot.changes());
        snapshot.watch(dirs);
        if !queue.is_empty() {
            progress.clear_console(console);
            progress.log(console, &message(&queue));
            continue;
        }
        if progress.shown() {
            console.out(&format!("{}\n", blue("Watching for changes...")));
        }
        while queue.is_empty() {
            thread::sleep(POLL);
            queue.extend(snapshot.changes());
        }
        progress.clear_console(console);
        if !progress.clears() {
            let text = "Rerunning tests due to changes...";
            let bar = "=".repeat(text.len());
            console.out(&format!(
                "{}\n",
                blue(&format!("\n\n\n{text}\n{bar}\n\n\n"))
            ));
        }
        progress.log(console, &message(&queue));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(event: Event, path: &str) -> Change {
        Change {
            event,
            path: path.to_owned(),
        }
    }

    #[test]
    fn changes_are_named_as_elm_test_names_them() {
        assert_eq!(
            message(&[change(Event::Changed, "tests/A.elm")]),
            "tests/A.elm changed"
        );
        assert_eq!(
            message(&[
                change(Event::Changed, "tests/A.elm"),
                change(Event::Added, "tests/B.elm"),
                change(Event::Changed, "tests/A.elm"),
            ]),
            "2 files added/changed"
        );
    }

    #[test]
    fn each_watched_path_is_walked_once() {
        let paths = [
            "/p/elm.json",
            "/p/tests",
            "/p/src",
            "/p/tests",
            "/p/tests/A",
            "/p",
        ];
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        assert_eq!(roots(&paths[..5]), paths[..3]);
        assert_eq!(roots(&paths), [PathBuf::from("/p")]);
        let ignored = [PathBuf::from("/p"), PathBuf::from("/p/node_modules/x")];
        assert_eq!(roots(&ignored), ignored);
    }

    #[test]
    fn snapshots_see_what_starts_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("elm.json"), "{}").unwrap();
        let mut snapshot = Snapshot::new(root);
        assert!(snapshot.changes().is_empty());
        fs::create_dir_all(root.join("tests/elm-stuff")).unwrap();
        fs::write(root.join("tests/elm-stuff/X.elm"), "").unwrap();
        fs::write(root.join("tests/A.elm"), "a").unwrap();
        fs::write(root.join("tests/notes.txt"), "a").unwrap();
        let tests = Path::new("tests").display().to_string();
        let a = Path::new("tests").join("A.elm").display().to_string();
        assert_eq!(
            snapshot.changes(),
            [change(Event::Added, &tests), change(Event::Added, &a)]
        );
        fs::write(root.join("tests/A.elm"), "ab").unwrap();
        fs::remove_file(root.join("elm.json")).unwrap();
        assert_eq!(
            snapshot.changes(),
            [
                change(Event::Changed, &a),
                change(Event::Removed, "elm.json")
            ]
        );
        // A directory watched from now on holds no changes yet.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/B.elm"), "b").unwrap();
        snapshot.watch(vec![root.join("src")]);
        assert!(snapshot.changes().is_empty());
    }
}
