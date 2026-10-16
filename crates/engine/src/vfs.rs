//! The files nomi reads and writes: the disk, with the text an editor
//! holds laid over it. Every change makes a new [`Revision`]; readers take
//! a [`Snapshot`], whose overlays never change under them and which knows
//! when a newer revision has made its work stale. Each file is named by
//! one [`FileId`], from its path with symbolic links resolved (R7).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard};

use nomi_base::{FileId, lock};
use walkdir::WalkDir;

/// Directories discovery never enters, as elm-format skips them: packages
/// and build products, and git's store.
const SKIPPED: &[&str] = &["elm-stuff", "node_modules", ".git"];

/// The extension of an Elm module.
const ELM: &str = "elm";

/// A state of the files: each overlay set or removed, and each change on
/// disk nomi made or was told of, makes the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

/// Where the engine's files come from and go to.
#[derive(Debug, Default)]
pub struct Vfs {
    files: Arc<Files>,
    /// The latest revision, which snapshots compare theirs with.
    latest: Arc<AtomicU64>,
    /// The editor's text, by file. Replaced, never changed, so snapshots
    /// share it.
    overlays: Mutex<Arc<HashMap<FileId, Arc<str>>>>,
}

/// The files given ids, which only grow, so every snapshot shares them.
#[derive(Debug, Default)]
struct Files(RwLock<Interner>);

#[derive(Debug, Default)]
struct Interner {
    ids: HashMap<PathBuf, FileId>,
    paths: Vec<Arc<Path>>,
    /// Each path as it was given, so that a file asked for again needn't
    /// be resolved on disk again: a build asks for each import's.
    given: HashMap<PathBuf, FileId>,
}

/// The files as they were at one revision. The editor's text is frozen;
/// the disk is read when asked, and a change to it that nomi made or was
/// told of makes the snapshot stale.
#[derive(Clone, Debug)]
pub struct Snapshot {
    revision: Revision,
    files: Arc<Files>,
    latest: Arc<AtomicU64>,
    overlays: Arc<HashMap<FileId, Arc<str>>>,
}

impl Files {
    fn id(&self, path: &Path) -> io::Result<FileId> {
        let known = self.read().given.get(path).copied();
        if let Some(id) = known {
            return Ok(id);
        }
        let given = path;
        let path = canonical(path)?;
        let mut interner = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = interner.ids.get(&path) {
            interner.given.insert(given.to_owned(), id);
            return Ok(id);
        }
        let raw = u32::try_from(interner.paths.len())
            .map_err(|_| io::Error::other("more files than a FileId can name"))?;
        let id = FileId::from_raw(raw);
        interner.paths.push(Arc::from(path.as_path()));
        interner.ids.insert(path, id);
        interner.given.insert(given.to_owned(), id);
        Ok(id)
    }

    fn path(&self, id: FileId) -> Arc<Path> {
        self.read().paths[id.raw() as usize].clone()
    }

    /// The interner, read even after a writer panicked, as writers only
    /// add to it.
    fn read(&self) -> RwLockReadGuard<'_, Interner> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `path` made absolute with every symbolic link resolved, so that one file
/// has one name. Of a path that doesn't exist yet, as an editor's new
/// buffer, the longest part that does is resolved and the rest kept.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    match fs::canonicalize(path).map(plain) {
        Ok(path) => return Ok(path),
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        Err(_) => {}
    }
    // Resolved a component at a time while they exist, and lexically
    // below one that doesn't, until `..` climbs back out of it.
    let mut resolved = PathBuf::new();
    // How many of the last components don't exist.
    let mut missing = 0;
    for component in std::path::absolute(path)?.components() {
        match component {
            Component::CurDir => {}
            // `C:` alone names a drive's working directory, not its root.
            Component::Prefix(_) | Component::RootDir => resolved.push(component),
            Component::ParentDir if missing > 0 => {
                resolved.pop();
                missing -= 1;
            }
            _ if missing == 0 => match fs::canonicalize(resolved.join(component)).map(plain) {
                Ok(path) => resolved = path,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    missing = 1;
                    resolved.push(component);
                }
                Err(e) => return Err(e),
            },
            _ => {
                missing += 1;
                resolved.push(component);
            }
        }
    }
    Ok(resolved)
}

/// [`canonical`], or `path` as given where that fails.
pub fn canonical_or_given(path: &Path) -> PathBuf {
    canonical(path).unwrap_or_else(|_| path.to_owned())
}

/// `path` without the `\\?\` prefix Windows' canonical paths have, where
/// the path means the same without it, as `C:\x` or `\\server\share`:
/// with it, a path makes no `file:` URL and starts no other path given
/// the usual way.
fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest)
            if rest.len() >= 3
                && rest.as_bytes()[0].is_ascii_alphabetic()
                && rest[1..].starts_with(":\\") =>
        {
            PathBuf::from(rest)
        }
        _ => path,
    }
}

impl Snapshot {
    /// The revision the files are at.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Whether a newer revision exists, so work on this one can stop.
    pub fn is_stale(&self) -> bool {
        self.latest.load(Ordering::Acquire) != self.revision.0
    }

    /// The id of the file at `path`.
    pub fn file_id(&self, path: &Path) -> io::Result<FileId> {
        self.files.id(path)
    }

    /// The path of `id`, with symbolic links resolved.
    pub fn path(&self, id: FileId) -> Arc<Path> {
        self.files.path(id)
    }

    /// The text of `id`: the editor's if it has the file open, else the
    /// disk's; a file on disk that isn't UTF-8 is an
    /// [`io::ErrorKind::InvalidData`] error.
    pub fn text(&self, id: FileId) -> io::Result<Arc<str>> {
        match self.overlays.get(&id) {
            Some(text) => Ok(text.clone()),
            None => Ok(Arc::from(fs::read_to_string(self.path(id))?)),
        }
    }

    /// The text of the file at `path`, as [`Snapshot::text`] gives it.
    pub fn read(&self, path: &Path) -> io::Result<String> {
        match self.overlay(path)? {
            Some(text) => Ok(text.to_string()),
            None => fs::read_to_string(path),
        }
    }

    /// The bytes of the file at `path`, for callers that report text that
    /// isn't UTF-8 themselves.
    pub fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.overlay(path)? {
            Some(text) => Ok(text.as_bytes().to_vec()),
            None => fs::read(path),
        }
    }

    /// Whether `path` names a file: one the editor has open, or one on
    /// disk, through any symbolic links.
    pub fn is_file(&self, path: &Path) -> bool {
        matches!(self.overlay(path), Ok(Some(_))) || path.is_file()
    }

    fn overlay(&self, path: &Path) -> io::Result<Option<&Arc<str>>> {
        if self.overlays.is_empty() {
            return Ok(None);
        }
        Ok(self.overlays.get(&self.file_id(path)?))
    }
}

impl Vfs {
    /// The disk, with nothing laid over it.
    pub fn new() -> Vfs {
        Vfs::default()
    }

    /// The files as they are now.
    pub fn snapshot(&self) -> Snapshot {
        // The overlays are read under their lock, so the revision and
        // the overlays agree.
        let overlays = lock(&self.overlays);
        Snapshot {
            revision: Revision(self.latest.load(Ordering::Acquire)),
            files: self.files.clone(),
            latest: self.latest.clone(),
            overlays: overlays.clone(),
        }
    }

    /// The latest revision.
    pub fn revision(&self) -> Revision {
        Revision(self.latest.load(Ordering::Acquire))
    }

    /// The id of the file at `path`.
    pub fn file_id(&self, path: &Path) -> io::Result<FileId> {
        self.files.id(path)
    }

    /// The path of `id`, with symbolic links resolved.
    pub fn path(&self, id: FileId) -> Arc<Path> {
        self.files.path(id)
    }

    /// Lays the editor's `text` over the file at `path`, which need not
    /// exist on disk, until [`Vfs::remove_overlay`].
    pub fn set_overlay(&self, path: &Path, text: impl Into<Arc<str>>) -> io::Result<Revision> {
        let id = self.file_id(path)?;
        let mut overlays = lock(&self.overlays);
        Arc::make_mut(&mut overlays).insert(id, text.into());
        Ok(self.next())
    }

    /// Returns the file at `path` to its text on disk.
    pub fn remove_overlay(&self, path: &Path) -> io::Result<Revision> {
        let id = self.file_id(path)?;
        let mut overlays = lock(&self.overlays);
        Arc::make_mut(&mut overlays).remove(&id);
        Ok(self.next())
    }

    /// Records that the disk changed, as a file watcher reports it, so
    /// snapshots taken before are stale.
    pub fn disk_changed(&self) -> Revision {
        let _overlays = lock(&self.overlays);
        self.next()
    }

    /// The next revision; the caller holds the overlays' lock.
    fn next(&self) -> Revision {
        Revision(self.latest.fetch_add(1, Ordering::AcqRel) + 1)
    }

    /// The text of the file at `path`, the editor's if it has the file
    /// open; a file on disk that isn't UTF-8 is an
    /// [`io::ErrorKind::InvalidData`] error.
    pub fn read(&self, path: &Path) -> io::Result<String> {
        self.snapshot().read(path)
    }

    /// The bytes of the file at `path`, as [`Vfs::read`] finds them, for
    /// callers that report text that isn't UTF-8 themselves.
    pub fn read_bytes(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.snapshot().read_bytes(path)
    }

    /// Replaces the file at `path` on disk, which must exist, with `text`
    /// in one step: the text goes to a new file beside it, which is then renamed
    /// over it, so a reader or an interruption sees the old file or the
    /// new, never part of one.
    ///
    /// Through a symbolic link, the file it names is replaced and the link
    /// kept. The file keeps its permissions; a hard link to it keeps the
    /// old text, since the new text is a new file.
    pub fn write(&self, path: &Path, text: impl AsRef<[u8]>) -> io::Result<()> {
        let target = if fs::symlink_metadata(path)?.file_type().is_symlink() {
            fs::canonicalize(path)?
        } else {
            path.to_owned()
        };
        let permissions = fs::metadata(&target)?.permissions();
        let dir = match target.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        let mut temp = tempfile::Builder::new().prefix(".nomi-").tempfile_in(dir)?;
        temp.write_all(text.as_ref())?;
        temp.as_file().set_permissions(permissions)?;
        temp.persist(&target).map_err(|e| e.error)?;
        self.disk_changed();
        Ok(())
    }

    /// Whether `path` names a file on disk, through any symbolic links.
    pub fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    /// Writes `text` to `path` as [`Vfs::write`] does, first creating the
    /// file, empty and with the default permissions, if it is missing.
    pub fn create(&self, path: &Path, text: impl AsRef<[u8]>) -> io::Result<()> {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        self.write(path, text)
    }
}

/// The Elm modules `root` names, as elm-format finds them: a file is
/// itself, whatever its extension, and a directory is every `.elm` file
/// under it, following symbolic links and skipping `elm-stuff`,
/// `node_modules` and `.git`, the root included. Files come depth
/// first, each directory's entries in the order of their names, where
/// elm-format takes the order the disk lists them in.
pub fn discover(root: &Path) -> io::Result<Vec<PathBuf>> {
    walk(root, Dangling::Kept)
}

/// The `.elm` files `root` names, as [`discover`] finds them but for its
/// links to nothing: elm reads no module that nothing imports, so these
/// are the modules a build or the editor lists.
pub(crate) fn elm_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = walk(root, Dangling::Skipped)?;
    files.retain(|path| is_elm(path));
    Ok(files)
}

/// The names of the directories in `dir`, through any symbolic links, in
/// order. The editor never holds a directory, so they are the disk's.
pub(crate) fn subdirs(dir: &Path) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().is_dir() {
            names.push(entry.file_name());
        }
    }
    names.sort();
    Ok(names)
}

/// What a walk makes of a link to nothing named `.elm`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dangling {
    Kept,
    Skipped,
}

fn walk(root: &Path, dangling: Dangling) -> io::Result<Vec<PathBuf>> {
    if !fs::metadata(root)?.is_dir() {
        return Ok(vec![root.to_owned()]);
    }
    let walk = WalkDir::new(root)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !(entry.file_type().is_dir() && skipped(entry.path())));
    let mut files = Vec::new();
    for entry in walk {
        match entry {
            Ok(entry) if entry.file_type().is_file() && is_elm(entry.path()) => {
                files.push(entry.into_path());
            }
            Ok(_) => {}
            // A link to nothing is a file to elm-format: one named `.elm`
            // fails when it is read, and any other is passed over.
            Err(e) if e.path().is_some_and(is_dangling) => {
                if let Some(path) = e.path().filter(|p| dangling == Dangling::Kept && is_elm(p)) {
                    files.push(path.to_owned());
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(files)
}

fn is_elm(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == ELM)
}

fn is_dangling(path: &Path) -> bool {
    path.is_symlink() && !path.exists()
}

fn skipped(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|name| SKIPPED.iter().any(|s| name == *s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_verbatim_prefixes_go_where_the_path_means_the_same() {
        let plain = |p: &str| plain(PathBuf::from(p)).to_string_lossy().into_owned();
        assert_eq!(plain(r"\\?\C:\work\src\Main.elm"), r"C:\work\src\Main.elm");
        assert_eq!(
            plain(r"\\?\UNC\server\share\Main.elm"),
            r"\\server\share\Main.elm"
        );
        // Without a drive it means something else, so it stays.
        assert_eq!(plain(r"\\?\Volume{1}\x"), r"\\?\Volume{1}\x");
        assert_eq!(plain("/home/elm/Main.elm"), "/home/elm/Main.elm");
    }
}
