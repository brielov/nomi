//! `cargo xtask oracle optimized`: the optimized graphs each pinned elm
//! writes for each optimizer case, stored for the compiler's tests.
//!
//! A case is a project as the modules oracle has them: `elm make`'s
//! arguments and files, with `default.elm.json` for an `elm.json` it
//! doesn't have, an application's `elm-version` rewritten to the elm's.
//! Each elm builds every case offline with its own `ELM_HOME`, holding
//! the corpus's elm/core, elm/html, elm/json and elm/virtual-dom, and the
//! `<Module>.elmo` files it leaves in `elm-stuff/` go to
//! `artifacts/<elm>/<case>/`; the dependencies' `o.dat` of the last case,
//! for the encoding's tests, to `artifacts/<elm>/`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use nomi_testkit::transcript::{Case, cases};

use super::modules::Recorder;
use super::remove_tree;
use super::sandbox::{corpus_releases, elmo_files};
use crate::corpus::compat;
use crate::references::References;

const CASES: &str = "crates/compiler/tests/optimized";

/// The packages every case may depend on.
const DEPENDENCIES: [&str; 4] = ["elm/core", "elm/html", "elm/json", "elm/virtual-dom"];

/// `cargo xtask oracle optimized`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = corpus_releases(root, &DEPENDENCIES)?;
    let default = fs::read_to_string(dir.join("default.elm.json"))?;
    let cases = cases(&dir.join("cases"));
    let artifacts = dir.join("artifacts");
    remove_tree(&artifacts)?;
    for compat in compat::compats(refs)? {
        let elm = Recorder::new(refs, "optimized", compat, &pinned)?;
        let out = artifacts.join(format!("elm-{compat}"));
        for (id, case) in &cases {
            let stuff =
                build(&elm, case, &default).with_context(|| format!("elm {compat} on {id}"))?;
            let to = out.join(id);
            fs::create_dir_all(&to)?;
            for (_, file) in elmo_files(&stuff)? {
                fs::copy(stuff.join(&file), to.join(&file))?;
            }
            fs::copy(stuff.join("o.dat"), out.join("o.dat"))?;
        }
        eprintln!("  elm {compat}: {} cases", cases.len());
    }
    println!("recorded {} cases", cases.len());
    Ok(())
}

/// Builds `case` with `elm` in a fresh project, and gives the
/// `elm-stuff/<release>` it left.
fn build(elm: &Recorder, case: &Case, default: &str) -> Result<PathBuf> {
    let (mut command, work) = elm.make(case, default)?;
    command
        .args(["--output=/dev/null", "--report=json"])
        .args(["+RTS", "-N1", "-RTS"]);
    let output = command.output().context("running elm")?;
    ensure!(
        output.status.success(),
        "elm rejected the case, which must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(work.join("elm-stuff").join(elm.compat.as_str()))
}
