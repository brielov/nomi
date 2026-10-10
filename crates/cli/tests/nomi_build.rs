//! `nomi build` (plan 8.13b): each case under `nomi_build/cases/` must
//! reproduce the transcript of the same name under `expected/`. nomi's
//! own messages have no oracle, so the transcripts are snapshots, read
//! when written: `NOMI_BLESS=1` writes them anew.
//!
//! Cases run offline, with a cache serving the packages vendored for the
//! compiler's tests.
#![cfg(unix)]

#[path = "support/website.rs"]
mod website;

use std::fs;
use std::path::Path;

use nomi_testkit::transcript::Case;
use website::{Website, offline};

#[test]
fn nomi_build_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_build");
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages).fill(cache.path());
    let bless = std::env::var_os("NOMI_BLESS").is_some();
    let mut paths: Vec<_> = fs::read_dir(dir.join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    let mut failures = Vec::new();
    for path in &paths {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let case = Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let work = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(work.path()).unwrap();
        case.create_files(&root).unwrap();
        let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let actual = case
            .run(offline(program, cache.path()), &root)
            .unwrap()
            .compared(&case, &root);
        let expected_path = dir.join("expected").join(format!("{name}.txt"));
        if bless {
            fs::write(&expected_path, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("{name}: {e}; run with NOMI_BLESS=1"));
        if actual != expected {
            failures.push(format!(
                "{name}\n--- expected\n{expected}--- actual\n{actual}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
