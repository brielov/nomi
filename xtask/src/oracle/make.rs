//! `cargo xtask oracle make`: each pinned elm's transcript of every case
//! of the CLI report matrix, stored for the CLI's `elm_make`
//! ratchet.
//!
//! A case is a project and `elm make`'s arguments, nearly all with
//! `--report=json`. Each elm runs offline on a fresh copy, in a directory
//! padded to 96 bytes, with an `ELM_HOME` holding the packages the CLI's
//! tests serve, and an application's `elm-version` rewritten to that
//! elm's where it names a pinned one. A transcript is compared as
//! [`Transcript::compared`] gives it, and `expected/<case>.txt` holds one
//! per distinct transcript, each after a `### ` line naming the elms that
//! wrote it.
//!
//! Each case runs a second time without `--report=json`, for the terminal
//! style's messages, into `expected_terminal/`.
//!
//! [`Transcript::compared`]: nomi_testkit::transcript::Transcript::compared

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use nomi_testkit::transcript::Case;

use super::modules::{canonical, case_files, padded, pinned, read_case};
use super::sandbox::{elm_home, offline};
use super::{binary_path, cache_dir};
use crate::corpus::compat::{self, Version};
use crate::corpus::parallel_on;
use crate::references::References;

pub(crate) const CASES: &str = "crates/cli/tests/elm_make";

/// The packages the CLI's tests serve, vendored for the compiler's.
const PACKAGES: [&str; 10] = [
    "elm/browser",
    "elm/bytes",
    "elm/core",
    "elm/file",
    "elm/html",
    "elm/http",
    "elm/json",
    "elm/time",
    "elm/url",
    "elm/virtual-dom",
];

/// Runs of one case before a lock failure is an error.
const RETRIES: u32 = 5;

/// Runs of one terminal case before a cut-short success is kept.
const CUT_SHORT_RETRIES: u32 = 20;

pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = pinned(root, &PACKAGES)?;
    let mut json = Vec::new();
    for (stem, path) in case_files(&dir.join("cases"))? {
        json.push((stem, read_case(&path)?));
    }
    let terminal: Vec<(String, Case)> = (json.iter())
        .map(|(stem, case)| (stem.clone(), terminal(case)))
        .collect();
    let cases: Vec<(String, Case)> = json.iter().chain(&terminal).cloned().collect();
    let versions = compat::compats(refs)?;
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut columns = Vec::new();
    for &version in &versions {
        let scratch = cache_dir()?
            .join("sandboxes")
            .join("make")
            .join(version.to_string());
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        let home = scratch.join("home");
        elm_home(&home, version, &pinned)?;
        let pin = (refs.binary.iter())
            .find(|b| b.name == "elm" && b.version == version.to_string())
            .with_context(|| format!("elm {version} is not pinned"))?;
        let elm = binary_path(pin)?;
        // One case first, so elm builds the packages before runs share them.
        let numbered: Vec<_> = cases.iter().enumerate().collect();
        let (first, rest) = numbered.split_at(1);
        let run = |(i, (id, case)): &(usize, &(String, Case))| {
            // By number: a case's name could take the padding's room.
            let work = scratch.join("work").join(i.to_string());
            record(&elm, &home, &work, version, case)
                .with_context(|| format!("elm {version} on {id}"))
        };
        let mut column = parallel_on(1, first, run);
        column.extend(parallel_on(jobs, rest, run));
        let column = column.into_iter().collect::<Result<Vec<_>>>()?;
        eprintln!("  elm {version}: {} cases", column.len());
        columns.push(column);
    }
    for dir in ["expected", "expected_terminal"].map(|d| dir.join(d)) {
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
    }
    for (i, (id, _)) in cases.iter().enumerate() {
        let expected = dir.join(if i < json.len() {
            "expected"
        } else {
            "expected_terminal"
        });
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
    println!("recorded {} cases", json.len());
    Ok(())
}

/// `case` in the terminal style: without `--report=json`.
pub(crate) fn terminal(case: &Case) -> Case {
    let mut case = case.clone();
    case.args.retain(|a| a != "--report=json");
    case
}

/// The case with an application's `elm-version` set to `version`, where
/// it names a pinned elm; a case about other versions keeps its own.
pub(crate) fn as_release(case: &Case, version: Version) -> Case {
    let mut case = case.clone();
    for (path, text) in &mut case.files {
        if path != "elm.json" || !text.contains("\"type\": \"application\"") {
            continue;
        }
        for pinned in ["0.19.1", "0.19.2", "0.19.3"] {
            let from = format!("\"elm-version\": \"{pinned}\"");
            *text = text.replace(&from, &format!("\"elm-version\": \"{version}\""));
        }
    }
    case
}

/// elm's transcript of `case` as `version`, run in `work`.
fn record(elm: &Path, home: &Path, work: &Path, version: Version, case: &Case) -> Result<String> {
    let case = as_release(case, version);
    let mut attempt = 1;
    loop {
        if work.exists() {
            fs::remove_dir_all(work)?;
        }
        fs::create_dir_all(work)?;
        let dir = PathBuf::from(canonical(&padded(work)?)?);
        case.create_files(&dir)?;
        case.date_files(&dir)?;
        let mut command = Command::new(elm);
        command.env("ELM_HOME", home).env("GHCRTS", "-N1");
        offline(&mut command);
        let transcript = case.run(command, &dir)?;
        // elm now and then trips over its own lock on `d.dat`.
        let locked = String::from_utf8_lossy(&transcript.stderr).contains("(file is locked)");
        // And in the terminal style, writing nothing, it may exit before
        // its build thread prints the last line, which ends with a newline.
        let terminal = !case.args.iter().any(|a| a == "--report=json");
        let cut_short = terminal
            && transcript.exit == Some(0)
            && !transcript.stdout.is_empty()
            && !transcript.stdout.ends_with(b"\n");
        let retries = if cut_short {
            CUT_SHORT_RETRIES
        } else {
            RETRIES
        };
        if !(locked || cut_short) || attempt >= retries {
            return Ok(transcript.compared(&case, &dir));
        }
        attempt += 1;
    }
}
