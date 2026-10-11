//! `cargo xtask oracle diff` and `oracle publish`: each pinned elm's
//! transcript of every case of `elm diff` and `elm bump` (plan 8.14), and
//! of `elm publish` failing (plan 8.14c), stored for the CLI's tests.
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
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};
use nomi_testkit::transcript::Case;

use super::modules::{canonical, case_files, padded, read_case};
use super::{binary_path, cache_dir};
use crate::corpus::compat;
use crate::download::{get_quiet, post_quiet};
use crate::references::References;

pub const CASES: &str = "crates/cli/tests/elm_diff";

pub const PUBLISH_CASES: &str = "crates/cli/tests/elm_publish";

/// `oracle diff`.
pub fn run(root: &Path, refs: &References) -> Result<()> {
    record_all(root, refs, CASES, "diff")
}

/// `oracle publish`.
pub fn publish(root: &Path, refs: &References) -> Result<()> {
    record_all(root, refs, PUBLISH_CASES, "publish")
}

fn record_all(root: &Path, refs: &References, cases_dir: &str, sandbox: &str) -> Result<()> {
    let dir = root.join(cases_dir);
    let mut cases = Vec::new();
    for (stem, path) in case_files(&dir.join("cases"))? {
        let case = read_case(&path)?;
        let git = (case.files.iter().map(|(p, _)| p))
            .chain(case.binaries.iter().map(|(p, _)| p))
            .any(|p| p == ".git" || p.starts_with(".git/") || p.contains("/.git/"));
        ensure!(
            !git,
            "{stem} holds a git repository, with which elm could publish for real"
        );
        cases.push((stem, case));
    }
    let versions = compat::compats(refs)?;
    let mut columns = Vec::new();
    let mut docs: BTreeSet<PathBuf> = BTreeSet::new();
    for old in [dir.join("docs"), dir.join("releases")] {
        if old.exists() {
            fs::remove_dir_all(old)?;
        }
    }
    for &version in &versions {
        let scratch = cache_dir()?
            .join("sandboxes")
            .join(sandbox)
            .join(version.to_string());
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        let home = scratch.join("home");
        fs::create_dir_all(&home)?;
        let pin = (refs.binary.iter())
            .find(|b| b.name == "elm" && b.version == version.to_string())
            .with_context(|| format!("elm {version} is not pinned"))?;
        let elm = binary_path(pin)?;
        ensure!(
            elm.exists(),
            "{} is missing; run `cargo xtask oracle fetch`",
            elm.display()
        );
        let mut column = Vec::new();
        // One at a time: they share the home, and the network.
        for (i, (id, case)) in cases.iter().enumerate() {
            let work = scratch.join("work").join(i.to_string());
            let transcript = record(&elm, &home, &work, case)
                .with_context(|| format!("elm {version} on {id}"))?;
            column.push(transcript);
        }
        eprintln!("  elm {version}: {} cases", column.len());
        // The docs it read or wrote.
        let packages = home.join(version.to_string()).join("packages");
        if packages.is_dir() {
            for file in crate::corpus::files_below(&packages)? {
                if file.ends_with("/docs.json") {
                    let to = dir.join("docs").join(version.to_string()).join(&file);
                    fs::create_dir_all(to.parent().unwrap())?;
                    fs::copy(packages.join(&file), &to)?;
                    docs.insert(PathBuf::from(&file));
                }
            }
        }
        columns.push(column);
    }
    let expected = dir.join("expected");
    if expected.exists() {
        fs::remove_dir_all(&expected)?;
    }
    fs::create_dir_all(&expected)?;
    for (i, (id, _)) in cases.iter().enumerate() {
        let mut groups: Vec<(Vec<String>, &str)> = Vec::new();
        for (version, column) in versions.iter().zip(&columns) {
            let elm = format!("elm {version}");
            match groups.iter_mut().find(|(_, t)| *t == column[i]) {
                Some((elms, _)) => elms.push(elm),
                None => groups.push((vec![elm], &column[i])),
            }
        }
        let mut out = String::new();
        for (elms, transcript) in groups {
            writeln!(out, "### {}", elms.join(", "))?;
            out.push_str(transcript);
        }
        fs::write(expected.join(format!("{id}.txt")), out)?;
    }
    // Each package whose docs were read: the releases the registry lists.
    let releases = dir.join("releases");
    let packages: BTreeSet<String> = (docs.iter())
        .filter_map(|file| {
            let parts: Vec<_> = file.to_str()?.split('/').collect();
            Some(format!("{}/{}", parts.first()?, parts.get(1)?))
        })
        .collect();
    for package in packages {
        let url = format!("https://package.elm-lang.org/packages/{package}/releases.json");
        let body = get_quiet(&url)?;
        let to = releases.join(format!("{package}.json"));
        fs::create_dir_all(to.parent().unwrap())?;
        fs::write(to, body)?;
    }
    let all = post_quiet("https://package.elm-lang.org/all-packages")?;
    let all: serde_json::Value = serde_json::from_slice(&all)?;
    let mut names: Vec<&String> = all
        .as_object()
        .context("a map of packages")?
        .keys()
        .collect();
    names.sort();
    let names: String = names.iter().map(|n| format!("{n}\n")).collect();
    fs::write(dir.join("packages.txt"), names)?;
    println!("recorded {} cases", cases.len());
    Ok(())
}

/// elm's transcript of `case`, run online in `work`.
fn record(elm: &Path, home: &Path, work: &Path, case: &Case) -> Result<String> {
    if work.exists() {
        fs::remove_dir_all(work)?;
    }
    fs::create_dir_all(work)?;
    let dir = PathBuf::from(canonical(&padded(work)?)?);
    case.create_files(&dir)?;
    case.date_files(&dir)?;
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
    let transcript = case.run_within(command, &dir, std::time::Duration::from_secs(120))?;
    Ok(transcript.compared(case, &dir))
}
