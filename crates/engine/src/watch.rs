//! Watching a project's files, as elm-test's `--watch` and `nomi run
//! --watch` do: elm-test watches with chokidar; nomi polls the watched
//! files' sizes and modification times, which for a project's few files
//! costs little and needs no dependency.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use walkdir::{DirEntry, WalkDir};

use crate::paths::{lexical, relative};

/// How often the files are looked at, and how long elm-test waits for
/// changes made together.
pub const POLL: Duration = Duration::from_millis(100);

/// Directories never looked in, elm-test's (`FindTests.ignoredDirsGlobs`).
pub const IGNORED: [&str; 2] = ["elm-stuff", "node_modules"];

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
pub struct Change {
    event: Event,
    /// `path.relative(projectRootDir, path)`.
    shown: String,
    /// The path, absolute.
    path: PathBuf,
}

impl Change {
    /// The path that changed, absolute.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What a path was when last looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    dir: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// The watched paths when last looked at.
#[derive(Debug)]
pub struct Snapshot {
    root: PathBuf,
    /// Paths watched whatever the project says, each a change itself,
    /// a directory by appearing.
    files: Vec<PathBuf>,
    /// The paths the project names: directories, whose `.elm` files are
    /// watched, and files.
    paths: Vec<PathBuf>,
    /// Directories every file of which is watched, as a dev server's
    /// static files are.
    every: Vec<PathBuf>,
    /// What is walked: the watched paths, each once and none another's
    /// walk sees.
    roots: Vec<PathBuf>,
    seen: BTreeMap<PathBuf, Seen>,
}

impl Snapshot {
    /// The project at `root`, watching `files` and, from
    /// [`Snapshot::watch`], what the project names.
    pub fn new(root: &Path, files: Vec<PathBuf>) -> Snapshot {
        let files = lexically(files);
        let mut snapshot = Snapshot {
            root: root.to_owned(),
            roots: roots(&files),
            files,
            paths: Vec::new(),
            every: Vec::new(),
            seen: BTreeMap::new(),
        };
        snapshot.seen = snapshot.look();
        snapshot
    }

    /// Watches `paths` from now on, and every file in the directories
    /// `every`, in place of those watched before, as `watcher.add` and
    /// `watcher.unwatch` do: what they hold already isn't a change, nor is
    /// what is no longer watched.
    pub fn watch(&mut self, paths: Vec<PathBuf>, every: Vec<PathBuf>) {
        let (paths, every) = (lexically(paths), lexically(every));
        if paths != self.paths || every != self.every {
            self.roots = roots(self.files.iter().chain(&paths).chain(&every));
            self.paths = paths;
            self.every = every;
            let roots = &self.roots;
            self.seen
                .retain(|path, _| roots.iter().any(|root| sees(root, path)));
            for (path, seen) in self.look() {
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

    /// The changes since last time.
    pub fn changes(&mut self) -> Vec<Change> {
        let now = self.look();
        let changes = self.diff(&now);
        self.seen = now;
        changes
    }

    /// What changed from what was seen to `now`: an `.elm` file, a file
    /// watched itself, or one of [`Snapshot::new`]'s directories appearing.
    fn diff(&self, now: &BTreeMap<PathBuf, Seen>) -> Vec<Change> {
        let mut changes = Vec::new();
        for (path, seen) in now {
            let event = match self.seen.get(path) {
                None => Event::Added,
                Some(was) if !seen.dir && !was.dir && was != seen => Event::Changed,
                Some(_) => continue,
            };
            // Of directories, only those watched whatever the project says
            // count, by appearing (`addDir`).
            if seen.dir && event == Event::Added && !self.files.contains(path) {
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
            || self.files.iter().chain(&self.paths).any(|p| p == path)
            || self.every.iter().any(|dir| path.starts_with(dir))
    }

    fn change(&self, event: Event, path: &Path) -> Change {
        Change {
            event,
            shown: relative(path, &self.root).display().to_string(),
            path: path.to_owned(),
        }
    }
}

/// Each of `paths` as [`lexical`] spells it, so that one path isn't
/// watched twice under two spellings.
fn lexically(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    for path in &mut paths {
        *path = lexical(path);
    }
    paths
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
/// names, following links.
fn walk(path: &Path, seen: &mut BTreeMap<PathBuf, Seen>) {
    let ignored =
        |entry: &DirEntry| entry.depth() > 0 && IGNORED.iter().any(|i| entry.file_name() == *i);
    let walk = WalkDir::new(path).follow_links(true).into_iter();
    for entry in walk.filter_entry(|entry| !ignored(entry)).flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let seen_now = Seen {
            dir: meta.is_dir(),
            size: meta.len(),
            modified: meta.modified().ok(),
        };
        seen.insert(entry.into_path(), seen_now);
    }
}

/// `watcherEventMessage`: what changed, in a few words.
pub fn message(queue: &[Change]) -> String {
    let mut paths: Vec<&str> = Vec::new();
    for change in queue {
        if !paths.contains(&change.shown.as_str()) {
            paths.push(&change.shown);
        }
    }
    if let [_] = paths.as_slice() {
        return format!("{} {}", queue[0].shown, queue[0].event.as_str());
    }
    let mut events: Vec<Event> = queue.iter().map(|c| c.event).collect();
    events.sort_unstable();
    events.dedup();
    let events: Vec<&str> = events.into_iter().map(Event::as_str).collect();
    format!("{} files {}", paths.len(), events.join("/"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A change to `path`, by `/`-separated components, below `root`.
    fn change(event: Event, root: &Path, path: &str) -> Change {
        let path: PathBuf = path.split('/').collect();
        Change {
            event,
            shown: path.display().to_string(),
            path: root.join(path),
        }
    }

    #[test]
    fn changes_are_named_as_elm_test_names_them() {
        let root = Path::new("/p");
        let a = change(Event::Changed, root, "tests/A.elm");
        assert_eq!(
            message(std::slice::from_ref(&a)),
            format!("{} changed", Path::new("tests").join("A.elm").display())
        );
        assert_eq!(
            message(&[a.clone(), change(Event::Added, root, "tests/B.elm"), a,]),
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
    fn snapshots_see_what_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("elm.json"), "{}").unwrap();
        let mut snapshot = Snapshot::new(root, vec![root.join("elm.json"), root.join("tests")]);
        assert!(snapshot.changes().is_empty());
        fs::create_dir_all(root.join("tests/elm-stuff")).unwrap();
        fs::write(root.join("tests/elm-stuff/X.elm"), "").unwrap();
        fs::write(root.join("tests/A.elm"), "a").unwrap();
        fs::write(root.join("tests/notes.txt"), "a").unwrap();
        assert_eq!(
            snapshot.changes(),
            [
                change(Event::Added, root, "tests"),
                change(Event::Added, root, "tests/A.elm")
            ]
        );
        fs::write(root.join("tests/A.elm"), "ab").unwrap();
        fs::remove_file(root.join("elm.json")).unwrap();
        assert_eq!(
            snapshot.changes(),
            [
                change(Event::Changed, root, "tests/A.elm"),
                change(Event::Removed, root, "elm.json")
            ]
        );
        // A path watched from now on holds no changes yet.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/B.elm"), "b").unwrap();
        fs::write(root.join("src/ffi.mjs"), "1").unwrap();
        fs::write(root.join("src/other.mjs"), "1").unwrap();
        fs::create_dir_all(root.join("public/css")).unwrap();
        fs::write(root.join("public/css/app.css"), "1").unwrap();
        snapshot.watch(
            vec![root.join("src"), root.join("src/ffi.mjs")],
            vec![root.join("public")],
        );
        assert!(snapshot.changes().is_empty());
        // A file watched itself counts whatever its extension, as does
        // every file in a directory watched whole.
        fs::write(root.join("src/ffi.mjs"), "22").unwrap();
        fs::write(root.join("src/other.mjs"), "22").unwrap();
        fs::write(root.join("public/css/app.css"), "22").unwrap();
        assert_eq!(
            snapshot.changes(),
            [
                change(Event::Changed, root, "public/css/app.css"),
                change(Event::Changed, root, "src/ffi.mjs")
            ]
        );
    }

    #[test]
    fn a_path_no_longer_watched_is_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in ["src/A.elm", "lib/B.elm"] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), "a").unwrap();
        }
        let mut snapshot = Snapshot::new(root, Vec::new());
        snapshot.watch(vec![root.join("src"), root.join("lib")], Vec::new());
        assert!(snapshot.changes().is_empty());
        snapshot.watch(vec![root.join("src")], Vec::new());
        assert!(snapshot.changes().is_empty());
        fs::write(root.join("lib/B.elm"), "bb").unwrap();
        assert!(snapshot.changes().is_empty());
    }

    #[test]
    fn a_path_spelt_two_ways_is_watched_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("apps/web")).unwrap();
        let mut snapshot = Snapshot::new(root, Vec::new());
        snapshot.watch(
            vec![root.join("apps/web/../../src"), root.join("./src")],
            Vec::new(),
        );
        fs::write(root.join("src/A.elm"), "a").unwrap();
        assert_eq!(
            snapshot.changes(),
            [change(Event::Added, root, "src/A.elm")]
        );
    }
}
