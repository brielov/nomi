//! What an editor shows of a project's problems: every module
//! under its source directories checked as `elm make` checks them, over
//! the editor's open buffers, with dependencies from the package store
//! alone. elm-language-server compiles the modules that define `main`,
//! or a package's exposed ones, and then every module those don't reach,
//! so every module is checked once, and so is every module here.

use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_registry::Store;

use crate::Snapshot;
use crate::artifacts::Artifacts;
use crate::check::{Build, Checked, Goal};
use crate::console::Personality;
use crate::index::{self, Memo, Setup};
use crate::names;
use crate::schedule::Cancelled;
use crate::vfs::canonical_or_given;

/// A project's problems.
#[derive(Debug)]
pub struct Live {
    /// The directory of its `elm.json`.
    pub root: PathBuf,
    /// The release it is checked as.
    pub compat: Compat,
    /// What checking it found.
    pub checked: Checked,
}

/// Checks the project whose `elm.json` is `manifest`, with `open` among
/// its modules where they lie in its source directories; none where the
/// project isn't readable or the store lacks a dependency, since then
/// nothing nomi says about it would be what elm says. What `artifacts`
/// holds of earlier checks is reused, as `nomi make` reuses it. Cancelled
/// once the snapshot is stale, when no further module is checked.
pub fn check(
    snapshot: &Snapshot,
    store: &Store,
    manifest: &Path,
    open: &[PathBuf],
    memo: &mut Memo,
    artifacts: Option<&Artifacts>,
) -> Result<Option<Live>, Cancelled> {
    let Some(setup) = Setup::read(snapshot, store, manifest).filter(|setup| setup.complete) else {
        return Ok(None);
    };
    let Some(env) = setup.env(&setup.exposing()) else {
        return Ok(None);
    };
    let Setup {
        root,
        outline,
        compat,
        sources,
        direct,
        ..
    } = setup;
    let Ok(packages) = names::package_interfaces_with(compat, &sources, memo) else {
        return Ok(None);
    };
    let visible = names::visible(&packages, &direct);
    let mut paths = index::elm_files(&env);
    for path in open {
        let path = canonical_or_given(path);
        let inside = env.src_dirs().iter().any(|dir| path.starts_with(dir));
        if inside && !paths.contains(&path) {
            paths.push(path);
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
    let build = Build {
        snapshot,
        env: &env,
        outline: &outline,
        paths: &paths,
        personality: Personality::Elm,
        packages: (&visible).into(),
        artifacts,
    };
    let checked = build.run(Goal::Check)?.checked;
    Ok(Some(Live {
        root,
        compat,
        checked,
    }))
}
