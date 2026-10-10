//! The JavaScript ratchet (plan 8.5): each fixture program of
//! `javascript/cases` in each mode as each release, passing when nomi
//! generates the bytes elm did, whose hashes `cargo xtask oracle
//! javascript` keeps in `expected.sha256`. A mode elm refuses, as
//! `--optimize` with `Debug`, isn't run.
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

use std::fs;
use std::path::Path;

use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::Case;

const MODES: [&str; 3] = ["dev", "debug", "optimize"];

#[test]
fn javascript() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/javascript");
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let default = fs::read_to_string(dir.join("default.elm.json")).unwrap();
    let suite = Suite::open(&dir);
    let mut paths: Vec<_> = fs::read_dir(dir.join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    let mut outcomes = Vec::new();
    for path in &paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let case = Case::parse(&fs::read_to_string(path).unwrap()).unwrap();
        for compat in projects::ALL {
            for mode in MODES {
                let id = format!("{name} {mode} as {compat}");
                let Some(expected) = suite.expected(&id) else {
                    continue;
                };
                let outcome = match projects::javascript(&case, compat, mode, &default, &packages) {
                    Ok(js) if expected.matches(&js) => Ok(()),
                    Ok(_) => Err("the JavaScript differs".to_owned()),
                    Err(why) => Err(why),
                };
                if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
                    eprintln!("FAIL {id}: {why}");
                }
                outcomes.push((id, outcome));
            }
        }
    }
    assert!(outcomes.len() >= 70, "only {} runs", outcomes.len());
    suite.finish(outcomes);
}
