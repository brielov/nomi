//! `cargo xtask oracle members`: each pinned elm's transcript of `elm
//! make` in each member of the full-stack example, stored for the CLI's
//! `elm_members` test.
//!
//! Each elm runs offline in a fresh copy of the example, with an
//! `ELM_HOME` holding the fixture packages the CLI's tests serve, and an
//! application's `elm-version` rewritten to that elm's. A member that
//! depends on no other builds; one that does names a package no registry
//! has. `expected/<member>.txt`, the member's path with `-` for `/`,
//! holds one transcript per distinct one, each after a `### ` line naming
//! the elms that wrote it.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use nomi_testkit::workspace::make_cases;

use super::make::record;
use super::modules::Recorder;
use super::sandbox::fixture_releases;
use super::{fresh_dir, recordings};
use crate::corpus::compat;
use crate::references::References;

const EXAMPLE: &str = "examples/fullstack";
const DIR: &str = "crates/cli/tests/elm_members";

/// `cargo xtask oracle members`.
pub(crate) fn run(root: &Path, refs: &References) -> Result<()> {
    let example = root.join(EXAMPLE);
    let cases = make_cases(&example);
    let pinned = fixture_releases()?;
    let compats = compat::compats(refs)?;
    let mut columns = Vec::new();
    for &compat in &compats {
        let elm = Recorder::new(refs, "members", compat, &pinned)?;
        let work = elm.scratch.join("work");
        let column = (cases.iter())
            .map(|(member, case)| {
                record(&elm, case, |case| {
                    fresh_dir(&work)?;
                    Ok(case.lay_out_in(&example, member, &work)?)
                })
                .with_context(|| format!("elm {compat} in {member}"))
            })
            .collect::<Result<Vec<_>>>()?;
        columns.push(column);
    }
    let expected = root.join(DIR).join("expected");
    fresh_dir(&expected)?;
    for (i, (member, _)) in cases.iter().enumerate() {
        let transcripts = compats.iter().copied().zip(columns.iter().map(|c| &*c[i]));
        let file = format!("{}.txt", member.replace('/', "-"));
        fs::write(expected.join(file), recordings(transcripts))?;
    }
    println!("recorded {} members", cases.len());
    Ok(())
}
