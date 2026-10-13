//! nomi invoked as `elm-test`, against elm-test 0.19.3-0's own transcripts
//! (plan 10.6a): each case under `elm_test/cases/` must reproduce the
//! transcript of the same name under `expected/`, which `cargo xtask
//! oracle transcripts elm-test` records, normalized as the recordings
//! are.
//!
//! Like the recordings, cases run offline, with a cache that holds the
//! fixture packages and nothing else. The tests run in Node.

#[path = "support/website.rs"]
mod website;

use std::fs;
use std::path::Path;
use std::process::Command;

use nomi_testkit::transcript::{Case, normalize_elm_test};
use website::{Website, offline};

#[test]
fn elm_test_transcripts() {
    if Command::new("node").arg("--version").output().is_err() && std::env::var_os("CI").is_none() {
        eprintln!("skipped: elm-test's runs need Node");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_test");
    let bin = tempfile::tempdir().unwrap();
    let elm_test = bin
        .path()
        .join(format!("elm-test{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm_test).unwrap();
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages).fill(cache.path());
    let mut cases = Vec::new();
    for entry in fs::read_dir(dir.join("cases")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if path.extension().is_some_and(|e| e == "case") {
            cases.push((name, path));
        }
    }
    cases.sort();
    assert!(cases.len() >= 30, "found only {} cases", cases.len());
    let mut failures = Vec::new();
    for (name, path) in &cases {
        let case = Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = fs::read_to_string(dir.join("expected").join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle transcripts`"));
        let work = tempfile::tempdir().unwrap();
        let work = nomi_engine::canonical(work.path()).unwrap();
        let root = work.join("project");
        case.create_files(&root).unwrap();
        let mut command = offline(&elm_test, cache.path());
        command.env_remove("FORCE_COLOR").env_remove("NO_COLOR");
        let mut transcript = case.run(command, &root).unwrap();
        transcript
            .files
            .retain(|(path, _)| !path.starts_with("elm-stuff/"));
        let dir = root.display().to_string();
        let home = cache.path().display().to_string();
        transcript.stdout = normalize_elm_test(&transcript.stdout, &dir, &home);
        transcript.stderr = normalize_elm_test(&transcript.stderr, &dir, &home);
        let actual = transcript.render();
        if actual != expected {
            failures.push(format!(
                "{name}:\n--- expected\n{expected}--- actual\n{actual}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} transcripts differ:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
