//! `nomi format`: every module under some paths, formatted in parallel on
//! the engine's pool.

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use nomi_base::{ColumnUnit, ElmPosition, LineIndex, TextLen};

use nomi_compiler::hir::Expr;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{Tool, parse_module, validate};

use crate::vfs::canonical_or_given;
use crate::{Pool, Vfs, discover};

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
pub struct Run {
    /// The paths that named no modules, and why.
    pub roots: Vec<(PathBuf, RootError)>,
    /// Each module, and what became of it.
    pub files: Vec<(PathBuf, Outcome)>,
}

impl Run {
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

/// `text` formatted, or the position elm-format reports it unparsable
/// at: its parser's, ported, where that rejects it too, and otherwise
/// the first error of nomi's own.
pub fn format_text(text: &str) -> Result<String, ElmPosition> {
    nomi_format::format_module(text).map_err(|error| match nomi_syntax::elm_format::parse(text) {
        Err(nomi_syntax::elm_format::Error::Position(at)) => at,
        Ok(()) | Err(nomi_syntax::elm_format::Error::Crash(_)) => {
            position(text, error.range.start())
        }
    })
}

/// How formatting `text`, a module `tool` accepts, changes what elm reads
/// in it: where its HIR's shape (`nomi_compiler::hir::shape`) first
/// differs, nothing where it reads the same or doesn't format.
/// elm-format reads a CRLF line break in a multiline string as
/// `\n` where elm reads both characters, a change nomi keeps
/// (`DECISIONS.md`), so a module with one is allowed it.
pub fn shape_change(text: &str, tool: Tool) -> Option<String> {
    let lower = |text: &str| {
        let parse = parse_module(text);
        let accepted = parse.errors().is_empty() && validate(&parse, tool).is_empty();
        let module = ast::Module::cast(parse.syntax()).expect("a module");
        accepted.then(|| nomi_compiler::hir::lower(&module))
    };
    let before = lower(text)?;
    let output = nomi_format::format(text)?;
    let Some(after) = lower(&output) else {
        return Some("elm rejects the formatted module".to_owned());
    };
    let crlf = (before.exprs.iter())
        .any(|(_, e)| matches!(e, Expr::Str(s) if s.starts_with("\"\"\"") && s.contains('\r')));
    nomi_compiler::hir::difference(&before, &after).filter(|_| !crlf)
}

/// `bytes` as text, or, where they aren't UTF-8, the position of the
/// first byte that isn't.
pub fn decode(bytes: Vec<u8>) -> Result<String, ElmPosition> {
    String::from_utf8(bytes).map_err(|e| {
        let valid = e.utf8_error().valid_up_to();
        let prefix = std::str::from_utf8(&e.as_bytes()[..valid]).expect("a valid prefix");
        position(prefix, prefix.text_len())
    })
}

/// Formats every module the `roots` name, as [`discover`] finds them;
/// a module named twice is formatted once.
pub fn format_paths(vfs: &Vfs, roots: &[PathBuf], mode: Mode) -> Run {
    let mut run = Run::default();
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for root in roots {
        match discover(root) {
            Ok(found) if found.is_empty() => run.roots.push((root.clone(), RootError::Empty)),
            Ok(found) => {
                files.extend(
                    (found.into_iter()).filter(|file| seen.insert(canonical_or_given(file))),
                );
            }
            Err(e) => run.roots.push((root.clone(), RootError::Io(e))),
        }
    }
    run.files = Pool::global().map(files, |file| {
        let outcome = match vfs.read(&file) {
            Ok(text) => rewrite(vfs, &file, &text, mode),
            Err(e) => Outcome::Failed(e),
        };
        (file, outcome)
    });
    run
}

/// What becomes of `text`, the module at `path`, formatted, and in
/// [`Mode::Write`] written back where that changes it.
pub(crate) fn rewrite(vfs: &Vfs, path: &Path, text: &str, mode: Mode) -> Outcome {
    let formatted = match format_text(text) {
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
