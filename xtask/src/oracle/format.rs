//! `cargo xtask oracle format`: elm-format's output for each formatter
//! fixture and each corpus module, stored for the format ratchets.
//!
//! The fixtures keep elm-format's output whole, under `expected/`, so a
//! mismatch shows as a diff. The corpus keeps a hash, and only for the
//! modules elm-format changes: most come back as they were.

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use nomi_testkit::files_below;
use nomi_testkit::sha256_hex as sha256;

use super::process::run_within;
use super::{generated, pinned_binary, remove_tree};
use crate::corpus::format::{self as corpus_format, Outcome};
use crate::corpus::syntax::{self as corpus_syntax, Sources};
use crate::references::References;
use nomi_testkit::parallel_on;

/// The fixture suite: inputs, elm-format's outputs and `passing.txt`.
const FIXTURES: &str = "crates/format/tests/format_fixtures";

/// Which inputs to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Inputs {
    Fixtures,
    Corpus,
    Both,
}

/// What elm-format made of one input.
#[derive(Debug)]
pub(crate) enum Formatted {
    Output(Vec<u8>),
    /// Where it stopped parsing, as `line:column`.
    Rejected(String),
    /// It failed some other way, saying this; 0.8.8 calls `error` on some
    /// inputs.
    Crashed(String),
}

/// `cargo xtask oracle format`.
pub(crate) fn run(root: &Path, refs: &References, inputs: Inputs, jobs: usize) -> Result<()> {
    let binary = pinned_binary(refs, "elm-format", None)?;
    if inputs != Inputs::Corpus {
        fixtures(root, &binary, jobs)?;
    }
    if inputs != Inputs::Fixtures {
        corpus(root, &binary, jobs)?;
    }
    Ok(())
}

fn fixtures(root: &Path, binary: &Path, jobs: usize) -> Result<()> {
    let dir = root.join(FIXTURES);
    let cases = files_below(&dir.join("inputs"))?;
    eprintln!("==> fixtures ({} inputs)", cases.len());
    let outcomes = parallel_on(jobs, &cases, |case| -> Result<Formatted> {
        let text = std::fs::read(dir.join("inputs").join(case))?;
        elm_format(binary, &text).with_context(|| case.clone())
    });
    let expected = dir.join("expected");
    remove_tree(&expected)?;
    let mut rejected = format!("{}\n", generated("format"));
    for (case, outcome) in cases.iter().zip(outcomes) {
        match outcome? {
            Formatted::Output(output) => {
                let path = expected.join(case);
                std::fs::create_dir_all(path.parent().expect("a file in a directory"))?;
                std::fs::write(path, output)?;
            }
            Formatted::Rejected(at) => writeln!(rejected, "{case}\t{at}")?,
            Formatted::Crashed(_) => writeln!(rejected, "{case}\t{}", corpus_syntax::CRASH)?,
        }
    }
    std::fs::write(dir.join("rejected.tsv"), rejected)?;
    Ok(())
}

fn corpus(root: &Path, binary: &Path, jobs: usize) -> Result<()> {
    let modules = corpus_syntax::modules(root)?;
    eprintln!("==> corpus ({} modules)", modules.len());
    let outcomes = parallel_on(jobs, &modules, |m| -> Result<Outcome> {
        let text = std::fs::read(&m.file)?;
        Ok(
            match elm_format(binary, &text).with_context(|| format!("{} {}", m.case, m.path))? {
                Formatted::Output(output) if output == text => Outcome::Same,
                Formatted::Output(output) => Outcome::Sha256(sha256(&output)),
                Formatted::Rejected(at) => Outcome::Rejected(at),
                Formatted::Crashed(_) => Outcome::Crashed,
            },
        )
    });
    let mut table = format!(
        "{}\n{}\ncase\tpath\toutput\n",
        generated("format"),
        Sources::of(&modules)?,
    );
    // Only modules elm-format changes or refuses.
    for (m, outcome) in modules.iter().zip(outcomes) {
        let outcome = outcome?;
        if outcome != Outcome::Same {
            writeln!(table, "{}\t{}\t{outcome}", m.case, m.path)?;
        }
    }
    let dir = root.join(corpus_format::DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("oracle.tsv"), table)?;
    Ok(())
}

/// Formats `text` as elm-format 0.8.8 does for Elm 0.19, from stdin.
pub(crate) fn elm_format(binary: &Path, text: &[u8]) -> Result<Formatted> {
    let mut command = Command::new(binary);
    command
        .args(["--stdin", "--elm-version=0.19"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_within(&mut command, text, None)
        .with_context(|| format!("running {}", binary.display()))?
        .expect("a run without a limit ends");
    if output.status.success() {
        return Ok(Formatted::Output(output.stdout));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(
        match stderr
            .strip_prefix("Unable to parse file <STDIN>:")
            .and_then(|rest| rest.split_whitespace().next())
        {
            Some(at) => Formatted::Rejected(at.to_owned()),
            None => Formatted::Crashed(format!("{}: {stderr}", output.status)),
        },
    )
}
