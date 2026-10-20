//! nomi/core's `Nomi.Bytes`, bundled with nomi: the elm-test suite in
//! `nomi_bytes/` checks each function against elm/bytes' encoders and
//! decoders on generated input, that UTF-8 is read strictly, refusing
//! each class of bad sequence, and that base64 and hex round-trip.

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;

use website::offline;

#[test]
fn the_bytes_suite_passes() {
    nomi_testkit::require_bun();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_bytes");
    nomi_testkit::copy_tree(&suite, &root, |_| true).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let cache = website::fixture_cache();
    let tested = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args([
            "test",
            "--offline",
            "--no-color",
            "--seed",
            "1",
            "--fuzz",
            "500",
        ])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&tested.stdout);
    assert_eq!(tested.status.code(), Some(0), "{out}{tested:?}");
    assert!(out.contains("Failed:   0\n"), "{out}");
}
