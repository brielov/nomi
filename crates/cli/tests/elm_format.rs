//! nomi invoked as `elm-format`, against elm-format 0.8.8's transcripts:
//! each case under `elm_format/cases/` must reproduce the transcript of
//! the same name under `expected/`, which `cargo xtask oracle transcripts`
//! records. Where nomi deviates on purpose, `deviations/` holds the case
//! with nomi's transcript beside it, by hand, and `DECISIONS.md` says why.

use std::fs;
use std::path::Path;
use std::process::Command;

use nomi_testkit::transcript::Case;

#[test]
fn elm_format_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_format");
    let bin = tempfile::tempdir().unwrap();
    let elm_format = bin
        .path()
        .join(format!("elm-format{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm_format).unwrap();
    let mut cases = Vec::new();
    for (inputs, expected) in [("cases", "expected"), ("deviations", "deviations")] {
        for entry in fs::read_dir(dir.join(inputs)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "case") {
                cases.push((
                    path.clone(),
                    dir.join(expected)
                        .join(path.with_extension("txt").file_name().unwrap()),
                ));
            }
        }
    }
    cases.sort();
    assert!(cases.len() > 50, "found only {} cases", cases.len());
    let mut failures = Vec::new();
    for (case_path, expected_path) in &cases {
        let name = case_path.file_stem().unwrap().to_string_lossy();
        let case = Case::parse(&fs::read_to_string(case_path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = fs::read_to_string(expected_path)
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle transcripts`"));
        let work = tempfile::tempdir().unwrap();
        case.create_files(work.path()).unwrap();
        let actual = case
            .run(Command::new(&elm_format), work.path())
            .unwrap()
            .render();
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
