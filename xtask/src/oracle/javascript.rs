//! `cargo xtask oracle javascript`: the JavaScript each pinned elm
//! generates for each fixture program in each mode, for the engine's
//! `javascript` ratchet.
//!
//! A case is a project as the modules oracle has them, `elm make`'s
//! arguments and files, with `default.elm.json` for an `elm.json` it
//! doesn't have and an application's `elm-version` rewritten to the
//! elm's. Its output is `out.js` unless its arguments name one, as an
//! HTML case does. Each elm builds every case offline with its own `ELM_HOME`
//! once in each mode: dev, `--debug` and `--optimize`. The outputs are
//! large, so `expected.sha256` keeps their hashes, as the ratchet reads
//! them, and the files stay in the sandbox for a look; a mode elm refuses
//! is left out, with why, in `refused.tsv`.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use nomi_testkit::sha256_hex;
use nomi_testkit::transcript::{Case, cases};

use super::generated;
use super::modules::Recorder;
use super::sandbox::{corpus_releases, first_title};
use crate::corpus::compat;
use crate::references::References;

const CASES: &str = "crates/engine/tests/javascript";

/// The packages the cases may depend on.
const DEPENDENCIES: [&str; 7] = [
    "elm/browser",
    "elm/core",
    "elm/html",
    "elm/json",
    "elm/time",
    "elm/url",
    "elm/virtual-dom",
];

/// How elm generates a program's JavaScript.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode {
    Dev,
    Debug,
    Optimize,
}

impl Mode {
    /// Every mode, in the order the oracles record them.
    pub(crate) const ALL: [Mode; 3] = [Mode::Dev, Mode::Debug, Mode::Optimize];

    /// The mode's name in a case's id and an output's file name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Dev => "dev",
            Mode::Debug => "debug",
            Mode::Optimize => "optimize",
        }
    }

    /// The flag `elm make` takes for the mode.
    pub(crate) fn flag(self) -> Option<&'static str> {
        match self {
            Mode::Dev => None,
            Mode::Debug => Some("--debug"),
            Mode::Optimize => Some("--optimize"),
        }
    }
}

/// What elm made of a case in a mode.
enum Built {
    /// The file it generated.
    Output(Vec<u8>),
    /// The title of the report refusing it.
    Refused(String),
}

/// `cargo xtask oracle javascript`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = corpus_releases(root, &DEPENDENCIES)?;
    let default = fs::read_to_string(dir.join("default.elm.json"))?;
    let cases = cases(&dir.join("cases"));
    let mut hashes = String::new();
    let mut refused = format!("{}\n", generated("javascript"));
    for compat in compat::compats(refs)? {
        let elm = Recorder::new(refs, "javascript", compat, &pinned)?;
        let out = elm.scratch.join("out");
        fs::create_dir_all(&out)?;
        for (id, case) in &cases {
            for mode in Mode::ALL {
                let run = format!("{id} {} as {compat}", mode.name());
                match build(&elm, case, &default, mode).with_context(|| format!("elm on {run}"))? {
                    Built::Output(js) => {
                        writeln!(hashes, "{}  {run}", sha256_hex(&js))?;
                        let extension = if output_of(case).ends_with(".html") {
                            "html"
                        } else {
                            "js"
                        };
                        fs::write(out.join(format!("{id}.{}.{extension}", mode.name())), &js)?;
                    }
                    Built::Refused(title) => writeln!(refused, "{run}\t{title}")?,
                }
            }
        }
        eprintln!("  elm {compat}: {} cases", cases.len());
    }
    fs::write(dir.join("expected.sha256"), hashes)?;
    fs::write(dir.join("refused.tsv"), refused)?;
    println!("recorded {} cases", cases.len());
    Ok(())
}

/// What `elm` generates for `case` in `mode`, in a fresh project.
fn build(elm: &Recorder, case: &Case, default: &str, mode: Mode) -> Result<Built> {
    let (mut command, work) = elm.make(case, default)?;
    let named = case.args.iter().any(|a| a.starts_with("--output="));
    command
        .args((!named).then_some("--output=out.js"))
        .arg("--report=json")
        .args(mode.flag())
        .args(["+RTS", "-N1", "-RTS"]);
    let output = command.output().context("running elm")?;
    if output.status.success() {
        return Ok(Built::Output(fs::read(work.join(output_of(case)))?));
    }
    let report: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap_or_default();
    Ok(Built::Refused(first_title(&report).to_owned()))
}

/// The file `case` makes: the one its arguments name, or `out.js`.
fn output_of(case: &Case) -> &str {
    (case.args.iter())
        .find_map(|a| a.strip_prefix("--output="))
        .unwrap_or("out.js")
}
