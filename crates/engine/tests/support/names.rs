//! The oracle's projects (`cargo xtask oracle names`, `oracle types`):
//! the curated cases, the seeds, and the seeds' mutations, each run as
//! every release, for the naming and type ratchets.
// Each test target uses only part of the support it shares.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_testkit::edit;
use nomi_testkit::transcript::Case;

use crate::projects::{self, ALL, CRASH, compat_name, table};

pub fn dir() -> PathBuf {
    suite("names")
}

/// The directory of the suite `name`.
pub fn suite(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

/// The fixture packages every project builds with (plan 5.3).
fn packages() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// Each project by its id: `case/…`, `seed/…` and `mutation/…`.
fn projects(dir: &Path) -> Vec<(String, Case)> {
    let read = |path: &Path| {
        Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let cases_in = |sub: &str| {
        let mut files: Vec<PathBuf> = fs::read_dir(dir.join(sub))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|path| {
                (
                    path.file_stem().unwrap().to_string_lossy().into_owned(),
                    read(&path),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut projects: Vec<(String, Case)> = cases_in("cases")
        .into_iter()
        .map(|(stem, case)| (format!("case/{stem}"), case))
        .collect();
    let seeds = cases_in("seeds");
    for (stem, case) in &seeds {
        projects.push((format!("seed/{stem}"), case.clone()));
    }
    let edits = edit::parse_table(&fs::read_to_string(dir.join("mutations.tsv")).unwrap()).unwrap();
    for edit in edits {
        let (_, seed) = seeds.iter().find(|(stem, _)| *stem == edit.seed).unwrap();
        projects.push((format!("mutation/{}", edit.id), edit.apply(seed)));
    }
    projects
}

/// What elm said and what nomi said about one project as one release.
pub struct Run {
    /// `<project id>/<release>`, the ratchet's case.
    pub id: String,
    pub elm_json: String,
    pub elm_terminal: String,
    pub json: String,
    pub terminal: String,
}

/// Every naming project as every release, but where elm crashed.
pub fn runs() -> Vec<Run> {
    runs_in(&dir())
}

/// Every project of the suite in `dir` as every release, but where elm
/// crashed.
pub fn runs_in(dir: &Path) -> Vec<Run> {
    let reports = table(&dir.join("reports.tsv"));
    let terminal = table(&dir.join("terminal.tsv"));
    let default = fs::read_to_string(dir.join("default.elm.json")).unwrap();
    let mut runs = Vec::new();
    for (id, case) in projects(dir) {
        for compat in ALL {
            let key = (id.clone(), compat_name(compat));
            let elm_json = reports[&key].clone();
            if elm_json == CRASH {
                continue;
            }
            let (json, nomi_terminal) = projects::run(&case, compat, &default, &packages());
            runs.push(Run {
                id: format!("{id}/{}", version(compat)),
                elm_json,
                elm_terminal: serde_json::from_str(&terminal[&key]).unwrap(),
                json,
                terminal: nomi_terminal,
            });
        }
    }
    runs
}

fn version(compat: Compat) -> String {
    compat.to_string()
}
