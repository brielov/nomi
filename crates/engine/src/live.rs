//! What an editor shows of a project's problems (plan 9.3): every module
//! under its source directories checked as `elm make` checks them, over
//! the editor's open buffers, with dependencies from the package store
//! alone. elm-language-server compiles the modules that define `main`,
//! or a package's exposed ones, and then every module those don't reach,
//! so every module is checked once, and so is every module here.

use std::path::{Path, PathBuf};

use nomi_base::Compat;

use crate::Snapshot;
use crate::check::{self, Checked};
use crate::console::Personality;
use crate::index::{self, Memo, Setup};
use crate::names;
use nomi_registry::Store;

/// A project's problems.
#[derive(Debug)]
pub struct Live {
    /// The directory of its `elm.json`.
    pub root: PathBuf,
    pub compat: Compat,
    pub checked: Checked,
}

/// Checks the project whose `elm.json` is `manifest`, with `open` among
/// its modules where they lie in its source directories; none where the
/// project isn't readable or the store lacks a dependency, since then
/// nothing nomi says about it would be what elm says.
pub fn check(
    snapshot: &Snapshot,
    store: &Store,
    manifest: &Path,
    open: &[PathBuf],
    memo: &mut Memo,
) -> Option<Live> {
    let Setup {
        root,
        outline,
        compat,
        env,
        sources,
        direct,
        complete,
    } = index::setup(snapshot, store, manifest)?;
    if !complete {
        return None;
    }
    let packages = names::package_interfaces_with(compat, &sources, memo).ok()?;
    let visible = names::visible(&packages, &direct);
    let mut paths = index::discover(&env);
    for path in open {
        let path = &crate::vfs::canonical(path).unwrap_or_else(|_| path.clone());
        let inside = env.src_dirs().iter().any(|dir| path.starts_with(dir));
        if inside && !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    // As elm-language-server gives them to `elm make`: relative to the
    // project, which reports quote.
    let mut paths: Vec<PathBuf> = (paths.into_iter())
        .map(|path| match path.strip_prefix(&root) {
            Ok(relative) => relative.to_owned(),
            Err(_) => path,
        })
        .collect();
    paths.sort();
    let env = env.with_cwd(&root);
    let checked = check::check(
        snapshot,
        &env,
        &outline,
        &paths,
        Personality::Elm,
        (&visible).into(),
    );
    Some(Live {
        root,
        compat,
        checked,
    })
}
