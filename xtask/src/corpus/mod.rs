//! `cargo xtask corpus`: real-world Elm code, pinned in `corpus/` and kept in
//! the development cache. Sources never enter the repository.

pub(crate) mod apps;
pub(crate) mod compat;
pub(crate) mod deps;
pub(crate) mod fixtures;
pub(crate) mod format;
pub(crate) mod hir;
pub(crate) mod lsp;
pub(crate) mod names;
pub(crate) mod outline;
pub(crate) mod packages;
pub(crate) mod runs;
pub(crate) mod syntax;
pub(crate) mod test_suites;
pub(crate) mod trace;

use std::path::{Path, PathBuf};

use anyhow::Result;
use nomi_testkit::{files_below, parallel_on};

use crate::oracle::{cache_dir, remove_tree};

/// What a project's copy for a language-server session or a test run
/// leaves out: what tools write, and Node version pins, `package.json`'s
/// engines among them, which would have a version manager on the path
/// fetch a Node to run elm-language-server or elm-test with.
const NOT_COPIED: [&str; 7] = [
    ".git",
    "elm-stuff",
    "node_modules",
    ".nvmrc",
    ".node-version",
    ".tool-versions",
    "package.json",
];

pub(crate) fn corpus_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("corpus"))
}

/// Runs `f` over `items` on a few threads and returns the results in order.
/// Downloads are I/O-bound; eight keeps the registry and GitHub comfortable.
pub(crate) fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    parallel_on(8, items, f)
}

/// `from` copied afresh to `to`, but for [`NOT_COPIED`].
pub(crate) fn copy_project(from: &Path, to: &Path) -> Result<()> {
    remove_tree(to)?;
    nomi_testkit::copy_tree(from, to, |path| {
        !NOT_COPIED.iter().any(|name| path.ends_with(name))
    })?;
    Ok(())
}

/// Whether the file at `path` within a corpus project is one of its Elm
/// modules, rather than something its tools wrote or fetched.
pub(crate) fn is_module(path: &str) -> bool {
    let tools = |part| part == "elm-stuff" || part == "node_modules";
    path.ends_with(".elm") && !path.starts_with(".git/") && !path.split('/').any(tools)
}

pub(crate) fn stats(root: &Path) -> Result<()> {
    let packages = packages::load(root)?;
    let dir = corpus_dir()?;
    let mut fetched = 0;
    let mut modules = 0;
    let mut lines = 0;
    for package in &packages.packages {
        let path = packages::package_dir(&dir, package);
        if !path.join(packages::STAMP).exists() {
            continue;
        }
        fetched += 1;
        let counts = elm_counts(&path)?;
        modules += counts.modules;
        lines += counts.lines;
    }
    println!(
        "packages  {} pinned, {} unavailable, {fetched} fetched, {modules} modules, {lines} lines",
        packages.packages.len(),
        packages.unavailable.len(),
    );
    for app in apps::load(root)?.app {
        let path = apps::app_dir(&dir, &app);
        if path.join(".git").exists() {
            let Counts { modules, lines } = elm_counts(&path)?;
            println!(
                "app       {:<14} {modules} modules, {lines} lines",
                app.name
            );
        } else {
            println!("app       {:<14} not fetched", app.name);
        }
    }
    compat::stats(root)
}

/// How many modules a project has and how many lines they hold.
struct Counts {
    modules: usize,
    lines: usize,
}

fn elm_counts(dir: &Path) -> Result<Counts> {
    let mut counts = Counts {
        modules: 0,
        lines: 0,
    };
    for file in files_below(dir)?.into_iter().filter(|f| is_module(f)) {
        counts.modules += 1;
        counts.lines += (std::fs::read(dir.join(&file))?.iter())
            .filter(|&&b| b == b'\n')
            .count();
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_are_elm_files_tools_didnt_write() {
        assert!(is_module("src/Main.elm"));
        assert!(!is_module("src/Main.js"));
        assert!(!is_module(".git/x.elm"));
        assert!(!is_module("elm-stuff/0.19.1/A.elm"));
        assert!(!is_module("tests/node_modules/b/A.elm"));
    }

    #[test]
    fn copies_leave_out_what_tools_write() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        for file in [
            "src/A.elm",
            "elm-stuff/x",
            "sub/package.json",
            "package.json.bak",
        ] {
            let path = from.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        copy_project(&from, &to).unwrap();
        assert_eq!(files_below(&to).unwrap(), ["package.json.bak", "src/A.elm"]);
    }
}
