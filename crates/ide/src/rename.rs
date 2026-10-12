//! Rename (plan 9.7) as elm-language-server does it (`renameProvider.ts`,
//! `renameUtils.ts`): every reference of the name under the cursor, its
//! declaration included, gets the new name in place of the old one, the
//! first place the reference writes it. Renaming a module also renames its
//! file. A name declared outside the project can't be renamed.

use std::path::{Path, PathBuf};

use nomi_base::{TextRange, TextSize};
use nomi_engine::Snapshot;
use nomi_engine::index::Index;
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::Edit;

/// What renaming changes: each file's edits, in the order the references
/// come, and the module's file moved, where a module is renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub files: Vec<(PathBuf, Vec<Edit>)>,
    pub moved: Option<(PathBuf, PathBuf)>,
}

/// Why a name can't be renamed, in elm-language-server's words.
pub const OUTSIDE: &str = "Can not rename, due to source being outside of you project.";

/// The range of the name at `offset` that renaming it would change; none
/// where it has no references.
pub fn prepare_rename(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Result<Option<TextRange>, &'static str> {
    let module = index.module(path).ok_or(OUTSIDE)?;
    let Some(token) = named_token(&module.root, offset) else {
        return Ok(None);
    };
    outside(snapshot, index, path, offset)?;
    let references = crate::references(snapshot, index, path, offset, true);
    if references.is_empty() {
        return Ok(None);
    }
    let parent = token.parent();
    // A qualifier's module, or a header's: its whole name.
    let whole = parent
        .filter(|p| matches!(p.kind(), SyntaxKind::Qualifier | SyntaxKind::ModuleName))
        .filter(|p| {
            p.kind() == SyntaxKind::Qualifier
                || p.parent()
                    .is_some_and(|h| h.kind() == SyntaxKind::ModuleHeader)
        });
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
) -> Result<Option<Rename>, &'static str> {
    let module = index.module(path).ok_or(OUTSIDE)?;
    let Some(token) = named_token(&module.root, offset) else {
        return Ok(None);
    };
    outside(snapshot, index, path, offset)?;
    let parent = token.parent();
    let header = parent
        .as_ref()
        .filter(|p| p.kind() == SyntaxKind::ModuleName)
        .filter(|p| {
            p.parent()
                .is_some_and(|h| h.kind() == SyntaxKind::ModuleHeader)
        });
    // A module's new name is capitalized, part by part.
    let new_name = match header {
        Some(_) => (new_name.split('.'))
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            })
            .collect::<Vec<_>>()
            .join("."),
        None => new_name.to_owned(),
    };
    // A module's name in a qualifier is all of it.
    let original = match parent
        .clone()
        .filter(|p| matches!(p.kind(), SyntaxKind::Qualifier | SyntaxKind::ModuleName))
    {
        Some(name) => {
            let parts: Vec<String> = (name.descendants_with_tokens())
                .filter_map(|e| e.into_token())
                .filter(|t| t.kind() == SyntaxKind::UpperIdent)
                .map(|t| t.text().to_owned())
                .collect();
            parts.join(".")
        }
        None => token.text().to_owned(),
    };
    let mut files: Vec<(PathBuf, Vec<Edit>)> = Vec::new();
    for location in crate::references(snapshot, index, path, offset, true) {
        let Ok(text) = snapshot.read(&location.path) else {
            continue;
        };
        let written = &text[location.range];
        let Some(at) = written.find(&original) else {
            continue;
        };
        let start = location.range.start() + TextSize::new(at as u32);
        let edit = Edit {
            range: TextRange::at(start, TextSize::of(original.as_str())),
            text: new_name.clone(),
        };
        match files.iter_mut().find(|(p, _)| *p == location.path) {
            Some((_, edits)) => edits.push(edit),
            None => files.push((location.path.clone(), vec![edit])),
        }
    }
    let moved = header.and_then(|_| {
        let source_dir = source_dir(index, path)?;
        let mut moved = source_dir;
        moved.extend(new_name.split('.'));
        moved.set_extension("elm");
        Some((path.to_owned(), moved))
    });
    Ok(Some(Rename { files, moved }))
}

/// Refuses a name declared outside the project.
fn outside(
    snapshot: &Snapshot,
    index: &Index,
    path: &Path,
    offset: TextSize,
) -> Result<(), &'static str> {
    match crate::definition(snapshot, index, path, offset) {
        Some(found) if index.module(&found.path).is_none() => Err(OUTSIDE),
        _ => Ok(()),
    }
}

/// The source directory a module's file is in, by its name.
fn source_dir(index: &Index, path: &Path) -> Option<PathBuf> {
    let module = index.module(path)?;
    let mut dir = path.to_owned();
    for _ in module.name.split('.') {
        dir.pop();
    }
    Some(dir)
}

/// The identifier at `offset`.
fn named_token(root: &SyntaxNode, offset: TextSize) -> Option<SyntaxToken> {
    root.token_at_offset(offset)
        .find(|t| matches!(t.kind(), SyntaxKind::LowerIdent | SyntaxKind::UpperIdent))
}

/// From a name's first capitalized part to its last.
fn upper_range(node: &SyntaxNode) -> TextRange {
    let uppers: Vec<TextRange> = (node.descendants_with_tokens())
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::UpperIdent)
        .map(|t| t.text_range())
        .collect();
    match (uppers.first(), uppers.last()) {
        (Some(first), Some(last)) => first.cover(*last),
        _ => node.text_range(),
    }
}
