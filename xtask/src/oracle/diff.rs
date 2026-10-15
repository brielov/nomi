//! `cargo xtask oracle diff` and `oracle publish`: each pinned elm's
//! transcript of every case of `elm diff` and `elm bump`, and
//! of `elm publish` failing, stored for the CLI's tests.
//!
//! No publish case may reach GitHub, past which elm would register the
//! package for real: none may hold a git repository, so each stops at
//! `git show`, if not before, and the oracle refuses one that does.
//!
//! Both commands insist on the latest registry (`Registry.latest`), so
//! unlike the other transcripts these are recorded online, with read-only
//! requests to package.elm-lang.org: each elm gets a fresh `ELM_HOME` and
//! downloads what it reads. A case runs in a fresh directory padded to 96
//! bytes, as `oracle make` runs them, and `expected/<case>.txt` holds one
//! transcript per distinct transcript, after a `### ` line naming the elms
//! that wrote it.
//!
//! What the recordings read is kept for nomi's side, which runs offline:
//! each release's `docs.json` under `docs/<elm>/`, per elm since an elm
//! writes the `docs.json` of each dependency it builds itself, in its own
//! order, and later reads that rather than the registry's; and each
//! package's `releases.json` under `releases/`; and every package's name,
//! a line each in `packages.txt`, which suggestions are drawn from.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use nomi_base::Compat;
use nomi_testkit::files_below;
use nomi_testkit::transcript::{Case, cases};

use super::{cache_dir, fresh_dir, padded_case, pinned_elm, recordings, remove_tree};
use crate::corpus::compat;
use crate::download::{get_quiet, post_quiet};
use crate::references::References;

const CASES: &str = "crates/cli/tests/elm_diff";

const PUBLISH_CASES: &str = "crates/cli/tests/elm_publish";

/// `cargo xtask oracle diff`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    record_all(root, refs, CASES, "diff")
}

/// `cargo xtask oracle publish`.
pub(crate) fn publish(root: &Path, refs: &References) -> Result<()> {
    record_all(root, refs, PUBLISH_CASES, "publish")
}

fn record_all(root: &Path, refs: &References, cases_dir: &str, sandbox: &str) -> Result<()> {
    let dir = root.join(cases_dir);
    let cases = cases(&dir.join("cases"));
    for (id, case) in &cases {
        let git = (case.files.iter().map(|(p, _)| p))
            .chain(case.binaries.iter().map(|(p, _)| p))
            .any(|p| p == ".git" || p.starts_with(".git/") || p.contains("/.git/"));
        ensure!(
            !git,
            "{id} holds a git repository, with which elm could publish for real"
        );
    }
    let compats = compat::compats(refs)?;
    let mut columns = Vec::new();
    let mut docs: BTreeSet<String> = BTreeSet::new();
    remove_tree(&dir.join("docs"))?;
    remove_tree(&dir.join("releases"))?;
    for &compat in &compats {
        let scratch = cache_dir()?
            .join("sandboxes")
            .join(sandbox)
            .join(compat.as_str());
        remove_tree(&scratch)?;
        let home = scratch.join("home");
        fs::create_dir_all(&home)?;
        let elm = pinned_elm(refs, compat)?;
        let mut column = Vec::new();
        // One at a time: they share the home, and the network.
        for (i, (id, case)) in cases.iter().enumerate() {
            let work = scratch.join("work").join(i.to_string());
            let transcript = record(&elm, &home, &work, case)
                .with_context(|| format!("elm {compat} on {id}"))?;
            column.push(transcript);
        }
        eprintln!("  elm {compat}: {} cases", column.len());
        // The docs it read or wrote; the newest elm's are what the
        // registry serves every elm.
        let packages = home.join(compat.as_str()).join("packages");
        if packages.is_dir() {
            for file in files_below(&packages)? {
                if file.ends_with("/docs.json") {
                    if compat == Compat::LATEST {
                        let to = dir.join("docs").join(compat.as_str()).join(&file);
                        fs::create_dir_all(to.parent().expect("a file in a directory"))?;
                        fs::copy(packages.join(&file), &to)?;
                    }
                    docs.insert(file);
                }
            }
        }
        columns.push(column);
    }
    let expected = dir.join("expected");
    fresh_dir(&expected)?;
    for (i, (id, _)) in cases.iter().enumerate() {
        let transcripts = compats.iter().copied().zip(columns.iter().map(|c| &*c[i]));
        fs::write(expected.join(format!("{id}.txt")), recordings(transcripts))?;
    }
    fetch_releases(&dir.join("releases"), &docs)?;
    fetch_names(&dir.join("packages.txt"))?;
    println!("recorded {} cases", cases.len());
    Ok(())
}

/// elm's transcript of `case`, run online in `work`.
fn record(elm: &Path, home: &Path, work: &Path, case: &Case) -> Result<String> {
    let dir = padded_case(work, case)?;
    let mut command = Command::new(elm);
    // A repository git finds above the sandbox mustn't serve as the
    // case's: with none, `elm publish` stops at `git show`.
    let no_git = work.join("no-git-here");
    command
        .env("ELM_HOME", home)
        .env("GHCRTS", "-N1")
        .env("GIT_DIR", &no_git)
        .env("GIT_CEILING_DIRECTORIES", work);
    // elm 0.19.3 deadlocks on its own package lock in some cases.
    let transcript = case.run_within(command, &dir, Duration::from_secs(120))?;
    Ok(transcript.compared(case, &dir))
}

/// The releases the registry lists of each package whose `docs.json`,
/// by its path in a package cache, the recordings read, in `dir`.
fn fetch_releases(dir: &Path, docs: &BTreeSet<String>) -> Result<()> {
    let packages: BTreeSet<String> = (docs.iter())
        .filter_map(|file| {
            let mut parts = file.split('/');
            Some(format!("{}/{}", parts.next()?, parts.next()?))
        })
        .collect();
    for package in packages {
        let url = format!("https://package.elm-lang.org/packages/{package}/releases.json");
        let body = get_quiet(&url)?;
        let to = dir.join(format!("{package}.json"));
        fs::create_dir_all(to.parent().expect("a file in a directory"))?;
        fs::write(to, body)?;
    }
    Ok(())
}

/// Every package's name, a line each, in order, in `file`.
fn fetch_names(file: &Path) -> Result<()> {
    let all = post_quiet("https://package.elm-lang.org/all-packages")?;
    let all: serde_json::Value = serde_json::from_slice(&all)?;
    let mut names: Vec<&String> = (all.as_object())
        .context("a map of packages")?
        .keys()
        .collect();
    names.sort();
    let names: String = names.iter().map(|n| format!("{n}\n")).collect();
    fs::write(file, names)?;
    Ok(())
}
