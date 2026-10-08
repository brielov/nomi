//! `nomi format`: every module under some paths, formatted in parallel on
//! the engine's pool.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nomi_base::{ColumnUnit, ElmPosition, LineIndex};

use crate::{Pool, Vfs};

/// What a run does with a module that isn't formatted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Rewrites it.
    Write,
    /// Only reports it.
    Check,
}

/// What became of one module.
#[derive(Debug)]
pub enum Outcome {
    /// It was formatted already.
    Unchanged,
    /// It wasn't: rewritten, or in [`Mode::Check`] left as it was.
    Formatted,
    /// elm-format rejects it; the position is its first error's.
    Invalid(ElmPosition),
    /// It couldn't be read or written.
    Failed(io::Error),
}

/// Why a path named no modules.
#[derive(Debug)]
pub enum RootError {
    /// It couldn't be listed.
    Io(io::Error),
    /// It is a directory without `.elm` files.
    Empty,
}

/// Everything a run did, in the order of the paths it was given.
#[derive(Debug, Default)]
pub struct Report {
    pub roots: Vec<(PathBuf, RootError)>,
    pub files: Vec<(PathBuf, Outcome)>,
}

impl Report {
    /// Whether the run found nothing wrong: every path named modules,
    /// each one valid, and in [`Mode::Check`] formatted already.
    pub fn succeeded(&self, mode: Mode) -> bool {
        self.roots.is_empty()
            && self.files.iter().all(|(_, outcome)| match outcome {
                Outcome::Unchanged => true,
                Outcome::Formatted => mode == Mode::Write,
                Outcome::Invalid(_) | Outcome::Failed(_) => false,
            })
    }
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::Io(e) => e.fmt(f),
            RootError::Empty => f.write_str("no .elm files"),
        }
    }
}

/// `text` formatted, or the position of its first error.
pub fn format_text(text: &str) -> Result<String, ElmPosition> {
    nomi_format::format_module(text).map_err(|error| position(text, error.range.start()))
}

/// Formats every module the `roots` name, as [`Vfs::discover`] finds them;
/// a module named twice is formatted once.
pub fn format_paths(vfs: &Vfs, roots: &[PathBuf], mode: Mode) -> Report {
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for root in roots {
        match vfs.discover(root) {
            Ok(found) if found.is_empty() => report.roots.push((root.clone(), RootError::Empty)),
            Ok(found) => files.extend(
                found
                    .into_iter()
                    .filter(|file| seen.insert(fs::canonicalize(file).unwrap_or(file.clone()))),
            ),
            Err(e) => report.roots.push((root.clone(), RootError::Io(e))),
        }
    }
    report.files = Pool::global().map(files, |file| {
        let outcome = format_file(vfs, &file, mode);
        (file, outcome)
    });
    report
}

fn format_file(vfs: &Vfs, path: &Path, mode: Mode) -> Outcome {
    let text = match vfs.read(path) {
        Ok(text) => text,
        Err(e) => return Outcome::Failed(e),
    };
    let formatted = match format_text(&text) {
        Ok(formatted) => formatted,
        Err(at) => return Outcome::Invalid(at),
    };
    if formatted == text {
        return Outcome::Unchanged;
    }
    if mode == Mode::Write
        && let Err(e) = vfs.write(path, &formatted)
    {
        return Outcome::Failed(e);
    }
    Outcome::Formatted
}

/// Where Elm puts `offset`; an offset the index can't place, which the
/// parser never reports, falls back to the start.
fn position(text: &str, offset: nomi_base::TextSize) -> ElmPosition {
    LineIndex::new(text)
        .elm_position(text, offset, ColumnUnit::CodePoint)
        .unwrap_or(ElmPosition { row: 1, column: 1 })
}
