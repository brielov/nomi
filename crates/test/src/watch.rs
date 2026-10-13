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

/// How often the files are looked at, and how long elm-test waits for
/// changes made together.
const POLL: Duration = Duration::from_millis(100);

/// Directories elm-test's watcher ignores (`FindTests.ignoredDirsGlobs`).
const IGNORED: [&str; 2] = ["elm-stuff", "node_modules"];

/// A change, as chokidar reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    event: &'static str,
    path: String,
}

/// What a file was when last looked at: whether a directory, its size and
/// its modification time.
type Seen = (bool, u64, Option<SystemTime>);

/// The watched paths when last looked at.
struct Snapshot {
    root: PathBuf,
    /// `elm.json` and `tests/`, watched whatever the project says.
    always: [PathBuf; 2],
    dirs: Vec<PathBuf>,
    seen: BTreeMap<PathBuf, Seen>,
}

impl Snapshot {
    fn new(root: &Path) -> Snapshot {
        let always = [root.join("elm.json"), root.join("tests")];
        let mut snapshot = Snapshot {
            root: root.to_owned(),
            always,
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
        for path in self.always.iter().chain(&self.dirs) {
            walk(path, &mut seen);
        }
        seen
    }

    /// The changes since last time that start a run: to an `.elm` file,
    /// `elm.json`, or `tests/` appearing.
    fn changes(&mut self) -> Vec<Change> {
        let now = self.look();
        let mut changes = Vec::new();
        for (path, &(dir, size, modified)) in &now {
            let event = match self.seen.get(path) {
                None => "added",
                Some(&(was_dir, was_size, was_modified))
                    if !dir && !was_dir && (was_size, was_modified) != (size, modified) =>
                {
                    "changed"
                }
                Some(_) => continue,
            };
            // Of directories, only `tests/` appearing counts (`addDir`).
            if dir && event == "added" && *path != self.always[1] {
                continue;
            }
            if dir || self.counts(path) {
                changes.push(self.change(event, path));
            }
        }
        for (path, &(dir, _, _)) in &self.seen {
            if !dir && !now.contains_key(path) && self.counts(path) {
                changes.push(self.change("removed", path));
            }
        }
        self.seen = now;
        changes
    }

    fn counts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "elm") || self.always.iter().any(|a| a == path)
    }

    /// `path.relative(projectRootDir, path)`.
    fn change(&self, event: &'static str, path: &Path) -> Change {
        Change {
            event,
            path: relative(&self.root, path),
        }
    }
}

/// `path` relative to `root`, through `..` where it isn't below it.
fn relative(root: &Path, path: &Path) -> String {
    let root: Vec<_> = root.components().collect();
    let path_parts: Vec<_> = path.components().collect();
    let common = (root.iter().zip(&path_parts))
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..root.len() {
        out.push("..");
    }
    for part in &path_parts[common..] {
        out.push(part.as_os_str());
    }
    out.display().to_string()
}

/// `path` and, for a directory, everything below it but the ignored
/// directories.
fn walk(path: &Path, seen: &mut BTreeMap<PathBuf, Seen>) {
    let Ok(meta) = fs::metadata(path) else { return };
    seen.insert(
        path.to_owned(),
        (meta.is_dir(), meta.len(), meta.modified().ok()),
    );
    if !meta.is_dir() {
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
        return format!("{} {}", queue[0].path, queue[0].event);
    }
    let mut events: Vec<&str> = queue.iter().map(|c| c.event).collect();
    events.sort_unstable();
    events.dedup();
    format!("{} files {}", paths.len(), events.join("/"))
}

/// One run: its exit code, and the test source directories it found.
pub(crate) type Once<'a> = dyn FnMut(&mut dyn Console, &mut Progress) -> (i32, Vec<PathBuf>) + 'a;

/// Runs the tests with `once` each time they change, forever: once gives
/// the test source directories to watch.
pub(crate) fn watch(
    console: &mut dyn Console,
    progress: &mut Progress,
    root: &Path,
    color: bool,
    once: &mut Once<'_>,
) -> i32 {
    let blue = |text: &str| {
        if color {
            format!("\x1b[34m{text}\x1b[39m")
        } else {
            text.to_owned()
        }
    };
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
        let (_, dirs) = once(console, progress);
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

    #[test]
    fn changes_are_named_as_elm_test_names_them() {
        let change = |event, path: &str| Change {
            event,
            path: path.to_owned(),
        };
        assert_eq!(
            message(&[change("changed", "tests/A.elm")]),
            "tests/A.elm changed"
        );
        assert_eq!(
            message(&[
                change("changed", "tests/A.elm"),
                change("added", "tests/B.elm"),
                change("changed", "tests/A.elm"),
            ]),
            "2 files added/changed"
        );
        assert_eq!(
            relative(Path::new("/p/app"), Path::new("/p/shared/A.elm")),
            Path::new("..")
                .join("shared")
                .join("A.elm")
                .display()
                .to_string()
        );
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
            [
                Change {
                    event: "added",
                    path: tests
                },
                Change {
                    event: "added",
                    path: a.clone()
                }
            ]
        );
        fs::write(root.join("tests/A.elm"), "ab").unwrap();
        fs::remove_file(root.join("elm.json")).unwrap();
        assert_eq!(
            snapshot.changes(),
            [
                Change {
                    event: "changed",
                    path: a
                },
                Change {
                    event: "removed",
                    path: "elm.json".to_owned()
                }
            ]
        );
        // A directory watched from now on holds no changes yet.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/B.elm"), "b").unwrap();
        snapshot.watch(vec![root.join("src")]);
        assert!(snapshot.changes().is_empty());
    }
}
