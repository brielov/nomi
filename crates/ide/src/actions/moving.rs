//! Moving a top-level function to another module
//! (`moveRefactoringHandler.ts`): the function goes, with its signature
//! and doc comment, to the end of the destination, and every module that
//! uses it imports it from there.

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode, child_token};

use super::refactor::{Commas, expose_edit, unexpose_edit};
use super::token_at;
use super::unused::{import_exposing, remove_from_import};
use crate::completion::Context;
use crate::navigation::Declaration;
use crate::tree::{child, code_range, written_imports};
use crate::{Edit, FileEdits};

/// A place a function can move to: one of the project's other modules,
/// by its file's name and folder, as written from the project's root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub name: String,
    pub folder: String,
    pub path: PathBuf,
}

/// An edit of a move: a usual one, or text added at a line, which may be
/// past the destination's end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveEdit {
    Edit(Edit),
    AtLine { line: u32, text: String },
}

/// `getMoveDestinations`: the project's modules but the one at `source`.
pub fn move_destinations(index: &Index, root: &Path, source: &Path) -> Vec<Destination> {
    let source = index.module(source).map(|m| m.path.clone());
    let mut found: Vec<Destination> = (index.modules.iter())
        .filter(|m| Some(&m.path) != source.as_ref())
        .filter_map(|m| {
            let relative = m.path.strip_prefix(root).ok()?;
            Some(Destination {
                name: relative.file_name()?.to_string_lossy().into_owned(),
                folder: relative.parent()?.to_string_lossy().into_owned(),
                path: m.path.clone(),
            })
        })
        .collect();
    found.sort_by(|a, b| (&a.folder, &a.name).cmp(&(&b.folder, &b.name)));
    found
}

/// How a reference to the moved function is written.
struct Use {
    path: PathBuf,
    /// The qualifier's first name, where the reference is qualified.
    qualifier: Option<TextRange>,
    /// Whether it's the function's own signature, definition or an
    /// exposing list's entry.
    declaring: bool,
}

/// `handleMoveRequest`: each file's edits that move the function named
/// at `offset` in `source` to `destination`.
pub fn move_function(
    snapshot: &Snapshot,
    index: &Index,
    source: &Path,
    offset: TextSize,
    destination: &Path,
) -> Option<FileEdits<MoveEdit>> {
    let module = index.module(source)?;
    let target = index.module(destination)?;
    let token = token_at(&module.root(), offset)?;
    let parent = token.parent()?;
    let declaration = match parent.kind() {
        SyntaxKind::Definition | SyntaxKind::Signature => parent.parent()?,
        _ => return None,
    };
    if declaration.kind() != SyntaxKind::ValueDecl {
        return None;
    }
    let definition = child(&declaration, SyntaxKind::Definition)?;
    let signature = child(&declaration, SyntaxKind::Signature);
    let (range, function) = moved_text(&module.text, &definition, signature.as_ref())?;
    let name = token.text();
    let declaring = [Some(&definition), signature.as_ref()];
    let at = token.text_range().start();
    let uses = uses(snapshot, index, source, at, module, &declaring);
    // The source's edits, then the destination's, each file once.
    let mut files = FileEdits(vec![
        (module.path.clone(), Vec::new()),
        (target.path.clone(), Vec::new()),
    ]);
    files
        .file(&module.path)
        .push(MoveEdit::Edit(Edit::delete(range)));
    let line = target.text.matches('\n').count() as u32 + 1;
    let text = function;
    files
        .file(&target.path)
        .push(MoveEdit::AtLine { line, text });
    let mut edit = |path: &Path, edit: Edit| files.file(path).push(MoveEdit::Edit(edit));
    if let Some(unexposed) = unexpose_edit(&module.root(), &module.text, name) {
        edit(&module.path, unexposed);
    }
    // Each file that uses it: its import of the old module loses it.
    let mut using: Vec<&Path> = Vec::new();
    for u in &uses {
        if !using.contains(&u.path.as_path()) {
            using.push(&u.path);
        }
    }
    for path in &using {
        if let Some(unimported) = index
            .module(path)
            .and_then(|o| unimport(o, &module.name, name))
        {
            edit(path, unimported);
        }
    }
    // The destination exposes it where others use it unqualified.
    if (uses.iter()).any(|u| u.path != target.path && u.qualifier.is_none())
        && let Some(exposed) = expose_edit(&target.root(), &target.text, name)
    {
        edit(&target.path, exposed);
    }
    // Qualified uses name the new module, or none in the destination.
    for u in &uses {
        if let Some(requalified) = u.requalified(target) {
            edit(&u.path, requalified);
        }
    }
    // And each file that uses it imports the new module.
    let source_uses = (uses.iter()).any(|u| u.path == module.path && !u.declaring);
    for path in using.into_iter().filter(|p| *p != target.path) {
        if path == module.path && !source_uses {
            continue;
        }
        let Some(other) = index.module(path) else {
            continue;
        };
        let expose = (uses.iter()).any(|u| u.path == path && u.qualifier.is_none());
        let context = Context::new(snapshot, index, other, path);
        edit(
            path,
            context.add_import(&target.name, None, expose.then_some(name)),
        );
    }
    Some(files)
}

/// The references to the function named at `at` in the file at `path`,
/// which `module` is, and how each is written: in `declaring`, its
/// definition and signature, or elsewhere.
fn uses(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    at: TextSize,
    module: &Indexed,
    declaring: &[Option<&SyntaxNode>],
) -> Vec<Use> {
    let all = Declaration::Include;
    (crate::references(snapshot, index, path, at, all).into_iter())
        .filter_map(|location| {
            let other = index.module(&location.path)?;
            let at = token_at(&other.root(), location.range.end() - TextSize::new(1))?;
            let declaring = at.parent_ancestors().any(|a| {
                a.kind() == SyntaxKind::ExposingList
                    || (other.path == module.path && declaring.contains(&Some(&a)))
            });
            let qualifier = (at.parent())
                .and_then(|p| child(&p, SyntaxKind::Qualifier))
                .and_then(|q| child_token(&q, SyntaxKind::UpperIdent))
                .map(|t| t.text_range());
            Some(Use {
                path: other.path.clone(),
                qualifier,
                declaring,
            })
        })
        .collect()
}

impl Use {
    /// Where it's qualified, the qualifier naming `target`, the module the
    /// function moves to, or none in that module.
    fn requalified(&self, target: &Indexed) -> Option<Edit> {
        let qualifier = self.qualifier?;
        Some(if self.path == target.path {
            Edit::delete(TextRange::new(
                qualifier.start(),
                qualifier.end() + TextSize::new(1),
            ))
        } else {
            Edit {
                range: qualifier,
                text: target.name.clone(),
            }
        })
    }
}

/// The function `definition` defines, with its signature and a comment
/// right before them: where it's written, and the text it moves as.
fn moved_text(
    text: &str,
    definition: &SyntaxNode,
    signature: Option<&SyntaxNode>,
) -> Option<(TextRange, String)> {
    let first = signature.unwrap_or(definition);
    let comment = std::iter::successors(first.first_token()?.prev_token(), |t| t.prev_token())
        .find(|t| !matches!(t.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .filter(|t| matches!(t.kind(), SyntaxKind::BlockComment | SyntaxKind::DocComment));
    let defined = code_range(definition)?;
    let start = match (&comment, signature) {
        (Some(comment), _) => comment.text_range().start(),
        (None, Some(signature)) => code_range(signature)?.start(),
        (None, None) => defined.start(),
    };
    let mut function = String::from("\n\n");
    if let Some(comment) = &comment {
        function.push_str(comment.text());
        function.push('\n');
    }
    if let Some(signature) = signature {
        function.push_str(&text[code_range(signature)?]);
        function.push('\n');
    }
    function.push_str(&text[defined]);
    Some((TextRange::new(start, defined.end()), function))
}

/// `name` out of `other`'s import of the module `from`.
fn unimport(other: &Indexed, from: &str, name: &str) -> Option<Edit> {
    let import = written_imports(&other.root()).find(|i| i.module == from)?;
    let (list, exposed) = import_exposing(&import.node);
    remove_from_import(&other.text, list.as_ref(), &exposed, name, Commas::Nearest)
}
