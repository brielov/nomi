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

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use nomi_testkit::transcript::{Case, cases};

use super::modules::Recorder;
use super::sandbox::{RETRIES, Retry, corpus_releases, locked, offline, retrying};
use super::{FIXTURE_PACKAGES, cores, fresh_dir, padded_case, recordings};
use crate::corpus::compat;
use crate::references::References;
use nomi_testkit::parallel_on;

const CASES: &str = "crates/cli/tests/elm_make";

/// Runs of one terminal case before a cut-short success is kept.
const CUT_SHORT_RETRIES: u32 = 20;

/// `cargo xtask oracle make`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = corpus_releases(root, &FIXTURE_PACKAGES)?;
    let json = cases(&dir.join("cases"));
    // Each case in the terminal style, without `--report=json`, after them.
    let terminal = json.iter().map(|(id, case)| {
        let mut case = case.clone();
        case.args.retain(|a| a != "--report=json");
        (id.clone(), case)
    });
    let cases: Vec<(String, Case)> = json.iter().cloned().chain(terminal).collect();
    let compats = compat::compats(refs)?;
    let mut columns = Vec::new();
    for &compat in &compats {
        let elm = Recorder::new(refs, "make", compat, &pinned)?;
        // One case first, so elm builds the packages before runs share them.
        let numbered: Vec<_> = cases.iter().enumerate().collect();
        let (first, rest) = numbered.split_at(1);
        let run = |(i, (id, case)): &(usize, &(String, Case))| {
            // By number: a case's name could take the padding's room.
            let work = elm.scratch.join("work").join(i.to_string());
            record(&elm, &work, case).with_context(|| format!("elm {compat} on {id}"))
        };
        let mut column = parallel_on(1, first, run);
        column.extend(parallel_on(cores(), rest, run));
        let column = column.into_iter().collect::<Result<Vec<_>>>()?;
        eprintln!("  elm {compat}: {} cases", column.len());
        columns.push(column);
    }
    let (expected, expected_terminal) = (dir.join("expected"), dir.join("expected_terminal"));
    fresh_dir(&expected)?;
    fresh_dir(&expected_terminal)?;
    for (i, (id, _)) in cases.iter().enumerate() {
        let to = if i < json.len() {
            &expected
        } else {
            &expected_terminal
        };
        let transcripts = compats.iter().copied().zip(columns.iter().map(|c| &*c[i]));
        fs::write(to.join(format!("{id}.txt")), recordings(transcripts))?;
    }
    println!("recorded {} cases", json.len());
    Ok(())
}

/// `elm`'s transcript of `case` as its release, run in `work`. A run
/// that trips over a lock [`RETRIES`] times, or is cut short
/// [`CUT_SHORT_RETRIES`] times, is recorded as it went.
fn record(elm: &Recorder, work: &Path, case: &Case) -> Result<String> {
    let case = case.clone().as_release(elm.compat);
    let (transcript, dir) = retrying(CUT_SHORT_RETRIES, |attempt, _| {
        let dir = padded_case(work, &case)?;
        let mut command = Command::new(&elm.elm);
        command.env("ELM_HOME", &elm.home).env("GHCRTS", "-N1");
        offline(&mut command);
        let transcript = case.run(command, &dir)?;
        // In the terminal style, writing nothing, elm may exit before its
        // build thread prints the last line, which ends with a newline.
        let terminal = !case.args.iter().any(|a| a == "--report=json");
        let cut_short = terminal
            && transcript.exit == Some(0)
            && !transcript.stdout.is_empty()
            && !transcript.stdout.ends_with(b"\n");
        let again = cut_short || locked(&transcript.stderr) && attempt < RETRIES;
        Ok(if again {
            Retry::Again((transcript, dir))
        } else {
            Retry::Done((transcript, dir))
        })
    })?;
    Ok(transcript.compared(&case, &dir))
}
