//! `cargo xtask oracle transcripts`: elm-format's transcript of each
//! command-line case under [`CASES`], stored under `expected/` for the
//! CLI's transcript test. elm-format runs in a scratch directory with its
//! own `ELM_HOME`, on one RTS thread, so files it formats concurrently
//! report in order.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};
use nomi_testkit::transcript::Case;

use super::{binary_path, cache_dir};
use crate::references::References;

/// The cases, beside `expected/` and the hand-written `deviations/`.
pub const CASES: &str = "crates/cli/tests/elm_format";

pub fn run(root: &Path, refs: &References) -> Result<()> {
    let pin = refs
        .binary
        .iter()
        .find(|b| b.name == "elm-format")
        .context("elm-format is not pinned")?;
    let binary = binary_path(pin)?;
    ensure!(
        binary.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        binary.display()
    );
    let dir = root.join(CASES);
    let expected = dir.join("expected");
    if expected.exists() {
        fs::remove_dir_all(&expected)?;
    }
    fs::create_dir_all(&expected)?;
    let scratch = cache_dir()?.join("sandboxes").join("transcripts");
    let (home, work) = (scratch.join("home"), scratch.join("work"));
    let mut names: Vec<_> = fs::read_dir(dir.join("cases"))?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    names.sort();
    eprintln!("==> {} transcripts", names.len());
    for name in &names {
        let stem = name
            .strip_suffix(".case")
            .with_context(|| format!("{name} is not a .case"))?;
        let case = Case::parse(&fs::read_to_string(dir.join("cases").join(name))?)
            .map_err(anyhow::Error::msg)
            .with_context(|| name.clone())?;
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        fs::create_dir_all(&home)?;
        fs::create_dir_all(&work)?;
        case.create_files(&work)?;
        let mut command = Command::new(&binary);
        command.env("ELM_HOME", &home).env("GHCRTS", "-N1");
        let transcript = case.run(command, &work).with_context(|| name.clone())?;
        fs::write(expected.join(format!("{stem}.txt")), transcript.render())?;
    }
    Ok(())
}
