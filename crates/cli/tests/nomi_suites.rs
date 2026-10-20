//! nomi/core's pure and stream modules, by elm-test suites `nomi test`
//! runs, each a project under `nomi_suites/`:
//!
//! - `bytes`: `Nomi.Bytes`, each function against elm/bytes' encoders
//!   and decoders on generated input, UTF-8 read strictly, refusing each
//!   class of bad sequence, and base64 and hex round-tripping.
//! - `stream`: `Nomi.Stream` and `Nomi.Stdio` on nomi/test's virtual
//!   clock: a slow writer holds back its producer, an interrupted read
//!   loses nothing, lines split across chunks, a line reader leaves the
//!   rest of the stream, reads are answered in order whether a task or the
//!   kernel makes the reader, and the fakes' stdio.

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;

use website::offline;

/// Runs the suite `name` with a fixed seed, which must pass.
fn passes(name: &str) {
    nomi_testkit::require_bun();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let suite = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/nomi_suites")
        .join(name);
    nomi_testkit::copy_tree(&suite, &root, |_| true).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let cache = website::fixture_cache();
    let tested = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args(["test", "--offline", "--no-color"])
        .args(["--seed", "1", "--fuzz", "500"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&tested.stdout);
    assert_eq!(tested.status.code(), Some(0), "{out}{tested:?}");
    assert!(out.contains("Failed:   0\n"), "{out}");
}

#[test]
fn the_bytes_suite_passes() {
    passes("bytes");
}

#[test]
fn the_stream_suite_passes() {
    passes("stream");
}
