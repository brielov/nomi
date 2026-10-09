//! `nomi check` against `elm make --output=/dev/null`, for the projects
//! in `tests/modules/cases` (`cargo xtask oracle modules` records them):
//! the same `--report=json`, and the same terminal report on standard
//! error, from each elm. Every case fails, if it does, in the phases nomi
//! has: the project, syntax, module names and imports.
//!
//! The reports were recorded on Linux. On Windows elm draws cycles in
//! ASCII and separates paths with `\`, as nomi does there, so they hold
//! on Unix only.
#![cfg(unix)]

#[path = "support/projects.rs"]
mod projects;

use std::fs;
use std::path::{Path, PathBuf};

use nomi_testkit::transcript::Case;

use projects::{ALL, compat_name, same_json, table};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/modules")
}

/// The fixture packages, whose sources make the interfaces (plan 5.3);
/// `tests/modules/packages` keeps only the `elm.json` the oracle had.
fn packages() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

#[test]
fn module_reports() {
    let reports = table(&dir().join("reports.tsv"));
    let terminal = table(&dir().join("terminal.tsv"));
    let default = fs::read_to_string(dir().join("default.elm.json")).unwrap();
    let mut cases: Vec<String> = fs::read_dir(dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    cases.sort();
    assert!(!cases.is_empty());
    let mut failures = Vec::new();
    for file in &cases {
        let stem = file.strip_suffix(".case").unwrap();
        let case = Case::parse(&fs::read_to_string(dir().join("cases").join(file)).unwrap())
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        for compat in ALL {
            let key = (stem.to_owned(), compat_name(compat));
            let (json, stderr) = projects::run(&case, compat, &default, &packages());
            let elm = &reports[&key];
            if !same_json(elm, &json) {
                failures.push(format!(
                    "{stem} ({compat}) --report=json:\nnomi:\n{json}\nelm:\n{elm}"
                ));
            }
            let elm: String = serde_json::from_str(&terminal[&key]).unwrap();
            if stderr != elm {
                failures.push(format!(
                    "{stem} ({compat}) terminal:\nnomi:\n{stderr}\nelm:\n{elm}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
