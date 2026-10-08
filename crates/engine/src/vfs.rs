//! The files nomi reads and writes. Today that is the disk; editor
//! overlays, revisions and `FileId`s join it in 4.1, behind the same type.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Directories discovery never enters, as elm-format skips them: packages
/// and build products, and git's store.
const SKIPPED: &[&str] = &["elm-stuff", "node_modules", ".git"];

/// The extension of an Elm module.
const ELM: &str = "elm";

/// Where the engine's files come from and go to.
#[derive(Debug, Default)]
pub struct Vfs {
    _disk: (),
}

impl Vfs {
    pub fn new() -> Vfs {
        Vfs::default()
    }

    /// The text of the file at `path`; a file that isn't UTF-8 is an
    /// [`io::ErrorKind::InvalidData`] error.
    pub fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    /// Replaces the file at `path`, which must exist, with `text` in one
    /// step: the text goes to a new file beside it, which is then renamed
    /// over it, so a reader or an interruption sees the old file or the
    /// new, never part of one.
    ///
    /// Through a symbolic link, the file it names is replaced and the link
    /// kept. The file keeps its permissions; a hard link to it keeps the
    /// old text, since the new text is a new file.
    pub fn write(&self, path: &Path, text: &str) -> io::Result<()> {
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
        temp.write_all(text.as_bytes())?;
        temp.as_file().set_permissions(permissions)?;
        temp.persist(&target).map_err(|e| e.error)?;
        Ok(())
    }

    /// The Elm modules `root` names, as elm-format finds them: a file is
    /// itself, whatever its extension, and a directory is every `.elm` file
    /// under it, following symbolic links and skipping `elm-stuff`,
    /// `node_modules` and `.git`, the root included. Files come depth
    /// first, each directory's entries in the order of their names, where
    /// elm-format takes the order the disk lists them in.
    pub fn discover(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
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
            let entry = entry?;
            let path = entry.path();
            if entry.file_type().is_file() && path.extension().is_some_and(|e| e == ELM) {
                files.push(entry.into_path());
            }
        }
        Ok(files)
    }
}

fn skipped(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|name| SKIPPED.iter().any(|s| name == *s))
}
