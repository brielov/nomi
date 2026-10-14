//! What the editor's file events change (`fileEventsHandler.ts`): a new,
//! empty module gets its header, and a module renamed or moved with its
//! folder is renamed wherever it's named.

use std::path::{Path, PathBuf};

use nomi_engine::Snapshot;
use nomi_engine::index::{self, Index};
use nomi_project::Outline;
use nomi_syntax::ast::{self, AstNode};

use crate::Edit;
use crate::tree::code_range;

/// The source directories of the project that owns `path`.
fn source_dirs(snapshot: &Snapshot, path: &Path) -> Vec<PathBuf> {
    let Some(manifest) = index::manifest(snapshot, path) else {
        return Vec::new();
    };
    let (Some(root), Ok(text)) = (manifest.parent(), snapshot.read(&manifest)) else {
        return Vec::new();
    };
    match Outline::detect(text.as_bytes()) {
        Ok((Outline::App(app), _)) => (app.source_dirs.iter())
            .filter_map(|d| d.path())
            .map(|d| root.join(d))
            .collect(),
        Ok((Outline::Pkg(_), _)) => vec![root.join("src")],
        Err(_) => Vec::new(),
    }
}

/// The name of the module at `path`, by where it is in its source
/// directory, where that makes a module's name.
fn module_name(snapshot: &Snapshot, path: &Path) -> Option<String> {
    let file = path.file_name()?;
    let resolved = nomi_engine::canonical(path.parent()?)
        .ok()
        .map(|p| p.join(file));
    let relative = source_dirs(snapshot, path).into_iter().find_map(|dir| {
        let resolved_dir = nomi_engine::canonical(&dir).ok();
        path.strip_prefix(&dir)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                resolved
                    .as_ref()?
                    .strip_prefix(resolved_dir?)
                    .ok()
                    .map(Path::to_path_buf)
            })
    })?;
    if relative.extension()? != "elm" {
        return None;
    }
    let parts: Vec<String> = (relative.with_extension("").iter())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // `^\p{Lu}[\p{L}\d_]*(?:\.\p{Lu}[\p{L}\d_]*)*$`
    let valid = parts.iter().all(|part| {
        let mut chars = part.chars();
        chars.next().is_some_and(char::is_uppercase)
            && chars.all(|c| c.is_alphabetic() || c.is_ascii_digit() || c == '_')
    });
    valid.then(|| parts.join("."))
}

/// The header an empty module just created at `path` gets, unless it
/// holds something already or its place makes no module's name.
pub fn created_module_header(snapshot: &Snapshot, path: &Path) -> Option<Edit> {
    let name = module_name(snapshot, path)?;
    let text = snapshot.read(path).ok()?;
    text.is_empty().then(|| Edit {
        range: nomi_base::TextRange::default(),
        text: format!("module {name} exposing (..)"),
    })
}

/// The edits renaming the file or folder at `old` to `new` takes: each
/// module in it renamed after its new place, wherever it's named.
pub fn renamed_files(
    snapshot: &Snapshot,
    index: &Index,
    old: &Path,
    new: &Path,
) -> Vec<(PathBuf, Vec<Edit>)> {
    if let Some(module) = index.module(old) {
        return renamed_module(snapshot, index, &module.path, new);
    }
    // A folder: each module below it.
    let olds: Vec<PathBuf> = [Some(old.to_path_buf()), nomi_engine::canonical(old).ok()]
        .into_iter()
        .flatten()
        .collect();
    let mut found: Vec<(PathBuf, Vec<Edit>)> = Vec::new();
    for module in &index.modules {
        let Some(rest) = olds.iter().find_map(|o| module.path.strip_prefix(o).ok()) else {
            continue;
        };
        for (path, edits) in renamed_module(snapshot, index, &module.path, &new.join(rest)) {
            match found.iter_mut().find(|(p, _)| *p == path) {
                Some((_, existing)) => existing.extend(edits),
                None => found.push((path, edits)),
            }
        }
    }
    found
}

fn renamed_module(
    snapshot: &Snapshot,
    index: &Index,
    old: &Path,
    new: &Path,
) -> Vec<(PathBuf, Vec<Edit>)> {
    let Some(name) = module_name(snapshot, new) else {
        return Vec::new();
    };
    let Some(module) = index.module(old) else {
        return Vec::new();
    };
    let at = (module.root().children().find_map(ast::ModuleHeader::cast))
        .and_then(|header| header.module_name())
        .and_then(|name| code_range(name.syntax()));
    let Some(at) = at else {
        return Vec::new();
    };
    match crate::rename(snapshot, index, old, at.start(), &name) {
        Ok(Some(rename)) => rename.files,
        _ => Vec::new(),
    }
}
