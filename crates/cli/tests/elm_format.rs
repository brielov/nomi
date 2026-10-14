//! nomi invoked as `elm-format`, against elm-format 0.8.8's transcripts:
//! each case under `elm_format/cases/` must reproduce the transcript of
//! the same name under `expected/`, which `cargo xtask oracle transcripts`
//! records. Where nomi deviates on purpose, `deviations/` holds the case
//! with nomi's transcript beside it, by hand, and `DECISIONS.md` says why.

#[path = "support/website.rs"]
pub mod website;

use std::path::Path;
use std::process::Command;

use nomi_testkit::transcript::{Case, check_transcripts};

#[test]
fn elm_format_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_format");
    let (_bin, elm_format) = website::personality("elm-format");
    let mut ran = 0;
    for (cases, expected) in [("cases", "expected"), ("deviations", "deviations")] {
        ran += check_transcripts(&dir.join(cases), &dir.join(expected), |_, case| {
            if !creatable(case) {
                return None;
            }
            let work = tempfile::tempdir().unwrap();
            case.create_files(work.path()).unwrap();
            let actual = case
                .run(Command::new(&elm_format), work.path())
                .unwrap()
                .render();
            // The transcripts were recorded on Linux; Windows separates the
            // paths of the files a directory holds with `\`.
            Some(if cfg!(windows) {
                (actual.split_inclusive('\n'))
                    .map(|line| match line.strip_prefix("|Processing file ") {
                        Some(path) => format!("|Processing file {}", path.replace('\\', "/")),
                        None => line.to_owned(),
                    })
                    .collect()
            } else {
                actual
            })
        });
    }
    assert!(ran > 50, "ran only {ran} cases");
}

/// Whether this platform can hold the case's files: Windows forbids some
/// characters in names, which cases about escaping paths need.
fn creatable(case: &Case) -> bool {
    !cfg!(windows)
        || (case.files.iter())
            .all(|(path, _)| !path.contains(['<', '>', ':', '"', '|', '?', '*', '\\']))
}
