//! `cargo xtask oracle optimized`: the optimized graphs each pinned elm
//! writes for each optimizer case, stored for the compiler's tests.
//!
//! A case is a project as the modules oracle has them: `elm make`'s
//! arguments and files, with `default.elm.json` for an `elm.json` it
//! doesn't have, an application's `elm-version` rewritten to the elm's.
//! Each elm builds every case offline with its own `ELM_HOME`, holding
//! the corpus's elm/core and elm/json, and the `<Module>.elmo` files it
//! leaves in `elm-stuff/` go to `artifacts/<elm>/<case>/`; the `o.dat`
//! of the dependencies, the same for every case, to `artifacts/<elm>/`.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use nomi_testkit::transcript::Case;

use super::modules::{case_files, pinned, read_case, with_elm_version};
use super::sandbox::{elm_home, offline};
use super::{binary_path, cache_dir};
use crate::corpus::compat::{self, Version};
use crate::references::References;

pub const CASES: &str = "crates/compiler/tests/optimized";

/// The packages every case may depend on.
const DEPENDENCIES: [&str; 2] = ["elm/core", "elm/json"];

pub fn run(root: &Path, refs: &References) -> Result<()> {
    let dir = root.join(CASES);
    let pinned = pinned(root, &DEPENDENCIES)?;
    let default = fs::read_to_string(dir.join("default.elm.json"))?;
    let mut cases = Vec::new();
    for (stem, path) in case_files(&dir.join("cases"))? {
        cases.push((stem, read_case(&path)?));
    }
    let artifacts = dir.join("artifacts");
    if artifacts.exists() {
        fs::remove_dir_all(&artifacts)?;
    }
    for version in compat::compats(refs)? {
        let scratch = cache_dir()?
            .join("sandboxes")
            .join("optimized")
            .join(version.to_string());
        if scratch.exists() {
            fs::remove_dir_all(&scratch)?;
        }
        let home = scratch.join("home");
        elm_home(&home, version, &pinned)?;
        let out = artifacts.join(format!("elm-{version}"));
        for (id, case) in &cases {
            let stuff = build(refs, &scratch, &home, version, case, &default)
                .with_context(|| format!("elm {version} on {id}"))?;
            let to = out.join(id);
            fs::create_dir_all(&to)?;
            for entry in fs::read_dir(&stuff)? {
                let path = entry?.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.ends_with(".elmo") {
                    fs::copy(&path, to.join(&*name))?;
                }
            }
            fs::copy(stuff.join("o.dat"), out.join("o.dat"))?;
        }
        eprintln!("  elm {version}: {} cases", cases.len());
    }
    println!("recorded {} cases", cases.len());
    Ok(())
}

/// Builds `case` with elm `version` in a fresh project in `scratch`, and
/// gives the `elm-stuff/<version>` it left.
fn build(
    refs: &References,
    scratch: &Path,
    home: &Path,
    version: Version,
    case: &Case,
    default: &str,
) -> Result<std::path::PathBuf> {
    let pin = (refs.binary.iter())
        .find(|b| b.name == "elm" && b.version == version.to_string())
        .with_context(|| format!("elm {version} is not pinned"))?;
    let elm = binary_path(pin)?;
    ensure!(
        elm.exists(),
        "{} is missing; run `cargo xtask oracle fetch`",
        elm.display()
    );
    let work = scratch.join("work");
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    case.create_files(&work)?;
    let manifest = work.join("elm.json");
    let text = fs::read_to_string(&manifest).unwrap_or_else(|_| default.to_owned());
    fs::write(&manifest, with_elm_version(&text, version))?;
    let mut command = Command::new(&elm);
    command
        .args(&case.args)
        .args(["--output=/dev/null", "--report=json"])
        .args(["+RTS", "-N1", "-RTS"])
        .current_dir(&work)
        .env("ELM_HOME", home);
    offline(&mut command);
    let output = command.output().context("running elm")?;
    if !output.status.success() {
        bail!(
            "elm rejected the case, which must compile:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(work.join("elm-stuff").join(version.to_string()))
}
