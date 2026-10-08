//! `cargo xtask corpus`: real-world Elm code, pinned in `corpus/` and kept in
//! the development cache. Sources never enter the repository.

pub mod apps;
pub mod compat;
pub mod deps;
pub mod format;
pub mod outline;
pub mod packages;
pub mod syntax;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;

use crate::oracle::cache_dir;

pub fn corpus_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("corpus"))
}

/// Runs `f` over `items` on a few threads and returns the results in order.
/// Downloads are I/O-bound; eight keeps the registry and GitHub comfortable.
pub fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    parallel_on(8, items, f)
}

/// [`parallel`] on `workers` threads.
pub fn parallel_on<T: Sync, R: Send>(
    workers: usize,
    items: &[T],
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = f(item);
                    results.lock().unwrap()[i] = Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

/// Every regular file below `root`, as sorted `/`-separated relative paths.
pub fn files_below(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root)?;
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The SHA-256 of a `sha256sum`-style listing of `files` under `root`, so a
/// whole generated tree is pinned by one hash.
pub fn tree_digest(root: &Path, files: &[String]) -> Result<String> {
    let mut listing = String::new();
    for file in files {
        let bytes = std::fs::read(root.join(file))?;
        listing.push_str(&crate::download::sha256(&bytes));
        listing.push_str("  ./");
        listing.push_str(file);
        listing.push('\n');
    }
    Ok(crate::download::sha256(listing.as_bytes()))
}

pub fn stats(root: &Path) -> Result<()> {
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
        let (m, l) = elm_counts(&path)?;
        modules += m;
        lines += l;
    }
    println!(
        "packages  {} pinned, {} unavailable, {fetched} fetched, {modules} modules, {lines} lines",
        packages.packages.len(),
        packages.unavailable.len(),
    );
    for app in apps::load(root)?.app {
        let path = apps::app_dir(&dir, &app);
        if path.join(".git").exists() {
            let (m, l) = elm_counts(&path)?;
            println!("app       {:<14} {m} modules, {l} lines", app.name);
        } else {
            println!("app       {:<14} not fetched", app.name);
        }
    }
    compat::stats(root)
}

/// Number of `.elm` files below `dir` and their total line count.
fn elm_counts(dir: &Path) -> Result<(usize, usize)> {
    let mut modules = 0;
    let mut lines = 0;
    for file in files_below(dir)? {
        if file.ends_with(".elm") && !file.starts_with(".git/") && !file.contains("/elm-stuff/") {
            modules += 1;
            lines += std::fs::read(dir.join(&file))?
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
        }
    }
    Ok((modules, lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_keeps_order() {
        let items: Vec<u32> = (0..100).collect();
        assert_eq!(
            parallel(&items, |x| x * 2),
            items.iter().map(|x| x * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tree_digest_is_a_sha256sum_listing() {
        let dir = std::env::temp_dir().join(format!("nomi-xtask-tree-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("sub/b.txt"), "b").unwrap();
        let files = files_below(&dir).unwrap();
        assert_eq!(files, ["a.txt", "sub/b.txt"]);
        let listing = format!(
            "{}  ./a.txt\n{}  ./sub/b.txt\n",
            crate::download::sha256(b"a"),
            crate::download::sha256(b"b")
        );
        assert_eq!(
            tree_digest(&dir, &files).unwrap(),
            crate::download::sha256(listing.as_bytes())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
