//! `cargo xtask refs`: puts each pinned source tree at its commit under
//! `reference/`. Git checks the commit hash, so a synced tree is the pinned one.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::references::{References, Source};

pub fn sync(root: &Path, refs: &References) -> Result<()> {
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
    let url = format!("https://github.com/{}", source.repo);
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir)?;
        git(dir, &["init", "--quiet"])?;
        git(dir, &["remote", "add", "origin", &url])?;
    } else if head(dir).as_deref() == Some(source.commit.as_str()) && clean(dir)? {
        eprintln!(
            "  reference/{} is at {} ({})",
            source.name,
            source.tag,
            short(&source.commit)
        );
        return Ok(());
    } else if !clean(dir)? {
        bail!("it has local changes; commit, stash or remove them first");
    } else {
        git(dir, &["remote", "set-url", "origin", &url])?;
    }
    eprintln!(
        "  reference/{}: fetching {} ({})",
        source.name,
        source.tag,
        short(&source.commit)
    );
    git(
        dir,
        &["fetch", "--quiet", "--depth", "1", "origin", &source.commit],
    )?;
    git(dir, &["checkout", "--quiet", "--detach", &source.commit])?;
    if head(dir).as_deref() != Some(source.commit.as_str()) {
        bail!("checkout did not land on {}", source.commit);
    }
    Ok(())
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

fn head(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn clean(dir: &Path) -> Result<bool> {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .context("running git status")?;
    Ok(out.status.success() && out.stdout.is_empty())
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
