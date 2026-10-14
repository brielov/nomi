//! `cargo xtask refs`: puts each pinned source tree at its commit under
//! `reference/`. Git checks the commit hash, so a synced tree is the pinned one.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::references::{References, Source};

/// Syncs every pinned source under `reference/`.
pub(crate) fn sync(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join("reference");
    for source in &refs.source {
        sync_one(&dir.join(&source.name), source)
            .with_context(|| format!("syncing reference/{}", source.name))?;
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !refs.source.iter().any(|s| s.name == name) {
                eprintln!("  reference/{name} is not pinned in references.toml; left alone");
            }
        }
    }
    Ok(())
}

fn sync_one(dir: &Path, source: &Source) -> Result<()> {
    let label = format!("reference/{} {}", source.name, source.tag);
    checkout(
        dir,
        &format!("https://github.com/{}", source.repo),
        &source.commit,
        &label,
    )
}

/// Puts the git repository at `dir` on `commit`, fetching only that commit.
/// Refuses to touch a tree with local changes.
pub(crate) fn checkout(dir: &Path, url: &str, commit: &str, label: &str) -> Result<()> {
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir)?;
        git(dir, &["init", "--quiet"])?;
        git(dir, &["remote", "add", "origin", url])?;
    } else if !clean(dir)? {
        bail!(
            "{} has local changes; commit, stash or remove them first",
            dir.display()
        );
    } else if head(dir).as_deref() == Some(commit) {
        eprintln!("  {label} is at {}", short(commit));
        return Ok(());
    } else {
        git(dir, &["remote", "set-url", "origin", url])?;
    }
    eprintln!("  {label}: fetching {}", short(commit));
    git(dir, &["fetch", "--quiet", "--depth", "1", "origin", commit])?;
    git(dir, &["checkout", "--quiet", "--detach", commit])?;
    if head(dir).as_deref() != Some(commit) {
        bail!("checkout did not land on {commit}");
    }
    Ok(())
}

/// The hash of `HEAD`'s tree, which pins the checked-out files exactly.
pub(crate) fn tree(dir: &Path) -> Result<String> {
    git_stdout(dir, &["rev-parse", "HEAD^{tree}"])
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

fn head(dir: &Path) -> Option<String> {
    git_stdout(dir, &["rev-parse", "HEAD"]).ok()
}

fn clean(dir: &Path) -> Result<bool> {
    Ok(git_stdout(dir, &["status", "--porcelain"])?.is_empty())
}

/// What git prints, trimmed, run with `args` in `dir`.
fn git_stdout(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("running git {}", args[0]))?;
    ensure!(
        out.status.success(),
        "git {} failed in {}",
        args.join(" "),
        dir.display()
    );
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .context("running git")?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}
