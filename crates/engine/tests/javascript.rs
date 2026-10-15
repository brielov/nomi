//! The JavaScript ratchet: each fixture program of
//! `javascript/cases` in each mode as each release, passing when nomi
//! generates the bytes elm did, whose hashes `cargo xtask oracle
//! javascript` keeps in `expected.sha256`. A mode elm refuses, as
//! `--optimize` with `Debug`, isn't run. What differs is kept in
//! `target/tmp/javascript`.
#![cfg(unix)]

#[path = "support/projects.rs"]
pub mod projects;

use std::fs;
use std::path::Path;

use nomi_base::Compat;
use nomi_engine::javascript::Kind;
use nomi_testkit::ratchet::{Suite, shown};
use nomi_testkit::transcript;

/// Each mode, as the cases' ids name it.
const MODES: [(&str, Kind); 3] = [
    ("dev", Kind::Dev),
    ("debug", Kind::Debug),
    ("optimize", Kind::Optimize),
];

#[test]
fn javascript() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/javascript");
    let default = fs::read_to_string(dir.join("default.elm.json")).unwrap();
    let suite = Suite::open(&dir);
    let mut outcomes = Vec::new();
    for (name, case) in transcript::cases(&dir.join("cases")) {
        for compat in Compat::ALL {
            for (mode, kind) in MODES {
                let id = format!("{name} {mode} as {compat}");
                let Some(expected) = suite.expected(&id) else {
                    continue;
                };
                let outcome = match projects::javascript(&case, compat, kind, &default) {
                    Ok(js) if expected.matches(&js) => Ok(()),
                    Ok(js) => {
                        // Kept for a look beside elm's, which the oracle
                        // leaves in its sandbox.
                        let kept = Path::new(env!("CARGO_TARGET_TMPDIR")).join("javascript");
                        fs::create_dir_all(&kept).unwrap();
                        fs::write(kept.join(format!("{name}.{mode}.{compat}.js")), js).unwrap();
                        Err("the JavaScript differs".to_owned())
                    }
                    Err(why) => Err(why),
                };
                outcomes.push(shown(id, outcome));
            }
        }
    }
    assert!(outcomes.len() >= 70, "only {} runs", outcomes.len());
    suite.finish(outcomes);
}
