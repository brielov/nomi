//! Rename as elm-language-server does it (`renameProvider.ts`,
//! `renameUtils.ts`): every reference of the name under the cursor, its
//! declaration included, gets the new name in place of the old one, the
//! first place the reference writes it. Renaming a module also renames its
//! file. A name declared outside the project can't be renamed.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::graph::module_path;
use nomi_engine::index::{Index, Indexed};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, tokens};

use crate::Edit;
use crate::navigation::Declaration;

/// What renaming changes: each file's edits, in the order the references
/// come, and the module's file moved, where a module is renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub files: Vec<(PathBuf, Vec<Edit>)>,
    pub moved: Option<Move>,
}

/// A file renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Why a name can't be renamed: it's declared outside the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutsideProject;

impl fmt::Display for OutsideProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // elm-language-server's words.
        f.write_str("Can not rename, due to source being outside of you project.")
    }
}

/// The range of the name at `offset` that renaming it would change; none
/// where it has no references.
pub fn prepare_rename(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Result<Option<TextRange>, OutsideProject> {
    let Some((_, token)) = renamed(snapshot, index, path, offset)? else {
        return Ok(None);
    };
    let references = crate::references(snapshot, index, path, offset, Declaration::Include);
    if references.is_empty() {
        return Ok(None);
    }
    // A qualifier's module, or a header's: its whole name.
    let whole = (token.parent()).filter(|p| p.kind() == SyntaxKind::Qualifier || in_header(p));
    Ok(Some(match whole {
        Some(name) => upper_range(&name),
        None => token.text_range(),
    }))
}

/// The edits that rename the name at `offset` to `new_name`.
pub fn rename(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
    new_name: &str,
) -> Result<Option<Rename>, OutsideProject> {
    let Some((module, token)) = renamed(snapshot, index, path, offset)? else {
        return Ok(None);
    };
    let parent = token.parent();
    let header = parent.as_ref().is_some_and(in_header);
    // A module's new name is capitalized, part by part.
    let new_name = if header {
        (new_name.split('.'))
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            })
            .collect::<Vec<_>>()
            .join(".")
    } else {
        new_name.to_owned()
    };
    // A module's name in a qualifier is all of it.
    let original = match parent
        .as_ref()
        .filter(|p| matches!(p.kind(), SyntaxKind::Qualifier | SyntaxKind::ModuleName))
    {
        Some(name) => {
            let parts: Vec<String> = tokens(name)
                .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                .map(|t| t.text().to_owned())
                .collect();
            parts.join(".")
        }
        None => token.text().to_owned(),
    };
    let mut files: Vec<(PathBuf, Vec<Edit>)> = Vec::new();
    // Each file's text, read once for each run of its references.
    let mut read: Option<(PathBuf, Arc<str>)> = None;
    for location in crate::references(snapshot, index, path, offset, Declaration::Include) {
        if read.as_ref().is_none_or(|(path, _)| *path != location.path) {
            read = text_of(snapshot, index, &location.path).map(|t| (location.path.clone(), t));
        }
        let Some((_, text)) = &read else {
            continue;
        };
        let Some(at) = text[location.range].find(&original) else {
            continue;
        };
        let start = location.range.start() + TextSize::new(at as u32);
        let edit = Edit {
            range: TextRange::at(start, TextSize::of(original.as_str())),
            text: new_name.clone(),
        };
        match files.iter_mut().find(|(p, _)| *p == location.path) {
            Some((_, edits)) => edits.push(edit),
            None => files.push((location.path, vec![edit])),
        }
    }
    let moved = header.then(|| {
        // The source directory the module's file is in, by its name.
        let mut dir = path.to_owned();
        for _ in module.name.split('.') {
            dir.pop();
        }
        Move {
            from: path.to_owned(),
            to: dir.join(module_path(&new_name, "elm")),
        }
    });
    Ok(Some(Rename { files, moved }))
}

/// The module at `path` and the name at `offset` in it to rename, where
/// the project declares the name.
fn renamed<'i>(
    snapshot: &Snapshot,
    index: &'i Index,
    path: &Path,
    offset: TextSize,
) -> Result<Option<(&'i Indexed, SyntaxToken)>, OutsideProject> {
    let module = index.module(path).ok_or(OutsideProject)?;
    let token = (module.root().token_at_offset(offset))
        .find(|t| matches!(t.kind(), SyntaxKind::LowerIdent | SyntaxKind::UpperIdent));
    let Some(token) = token else {
        return Ok(None);
    };
    match crate::definition(snapshot, index, path, offset) {
        Some(found) if index.module(&found.path).is_none() => Err(OutsideProject),
        _ => Ok(Some((module, token))),
    }
}

/// The text of the file at `path`: the index's, for one of the project's
/// modules, else as the snapshot has it.
fn text_of(snapshot: &Snapshot, index: &Index, path: &Path) -> Option<Arc<str>> {
    match index.modules.iter().find(|m| m.path == path) {
        Some(module) => Some(module.text.clone()),
        None => Some(snapshot.read(path).ok()?.into()),
    }
}

/// Whether `name` is the module's name in its header.
fn in_header(name: &SyntaxNode) -> bool {
    name.kind() == SyntaxKind::ModuleName
        && name
            .parent()
            .is_some_and(|h| h.kind() == SyntaxKind::ModuleHeader)
}

/// From a name's first capitalized part to its last.
fn upper_range(node: &SyntaxNode) -> TextRange {
    let mut uppers = tokens(node)
        .filter(|t| t.kind() == SyntaxKind::UpperIdent)
        .map(|t| t.text_range());
    match uppers.next() {
        Some(first) => first.cover(uppers.last().unwrap_or(first)),
        None => node.text_range(),
    }
}
