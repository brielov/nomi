//! `nomi build`: each case under `nomi_build/cases/` must
//! reproduce the transcript of the same name under `expected/`. nomi's
//! own messages have no oracle, so the transcripts are snapshots, read
//! when written: `NOMI_BLESS=1` writes them anew.
//!
//! Cases run offline, with a cache serving the packages vendored for the
//! compiler's tests.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;

use nomi_testkit::transcript::check_transcripts;
use website::offline;

#[test]
fn nomi_build_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_build");
    let cache = website::fixture_cache();
    let bless = std::env::var_os("NOMI_BLESS").is_some();
    check_transcripts(&dir.join("cases"), &dir.join("expected"), |name, case| {
        let work = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(work.path()).unwrap();
        case.create_files(&root).unwrap();
        let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let actual = case
            .run(offline(program, cache.path()), &root)
            .unwrap()
            .compared(case, &root);
        if bless {
            fs::write(dir.join("expected").join(format!("{name}.txt")), &actual).unwrap();
        }
        Some(actual)
    });
}

/// A project built once keeps its dependencies as solved until its
/// elm.json changes, as elm keeps `elm-stuff/d.dat`: the next build
/// reads no package list.
#[test]
fn a_built_project_keeps_its_dependencies_solved() {
    let cache = website::fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let outline = |core: &str| {
        format!(
            r#"{{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {{"direct": {{"elm/core": "{core}", "elm/json": "1.1.4"}}, "indirect": {{}}}}, "test-dependencies": {{"direct": {{}}, "indirect": {{}}}}}}"#
        )
    };
    fs::write(root.join("elm.json"), outline("1.0.5")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/Main.elm"),
        "module Main exposing (value)\n\nvalue =\n    1\n",
    )
    .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let build = || {
        let mut command = offline(program, cache.path());
        command.current_dir(&root).args(["check", "src/Main.elm"]);
        command.output().unwrap()
    };
    let first = build();
    assert!(first.status.success(), "{first:?}");
    let index = cache.path().join("registry/index");
    fs::remove_file(&index).unwrap();
    let second = build();
    assert!(second.status.success(), "{second:?}");
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&second.stdout)
    );
    // Changed, it is solved again, and without the package list that fails.
    fs::write(root.join("elm.json"), outline("1.0.4")).unwrap();
    let third = build();
    let said = String::from_utf8_lossy(&third.stdout) + String::from_utf8_lossy(&third.stderr);
    assert!(said.contains("PACKAGE LIST"), "{said}");
}
