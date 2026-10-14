//! Parity with elm-language-server 2.10.0: each case's session,
//! recorded from it by `cargo xtask oracle lsp`, replayed against `nomi
//! lsp` on an in-memory connection, and its diagnostics and each answer
//! compared under the ratchet.
//!
//! nomi gets the fixture packages the recording gave elm-language-server,
//! in a store of its own, and its paths to them are written as the
//! recording writes `ELM_HOME`'s.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_testkit::lsp::{self, Timing};
use nomi_testkit::ratchet::Suite;
use serde_json::Value;

mod common;

use common::{built, real, replay, store};

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_language_server")
}

#[test]
fn sessions_answer_as_elm_language_server_does() {
    let suite = Suite::open(suite_dir());
    let temp = tempfile::tempdir().unwrap();
    let (store, packages) = store(&temp.path().join("store"));
    let memo = built(temp.path(), &store);
    let cases = lsp::cases(&suite_dir().join("cases"));
    // A session spends most of its time waiting for its server to go
    // quiet, so many run side by side.
    let next = AtomicUsize::new(0);
    let threads = thread::available_parallelism().map_or(16, |n| 4 * n.get());
    let mut outcomes: Vec<_> = thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut outcomes = Vec::new();
                    while let Some(case) = cases.get(next.fetch_add(1, Ordering::Relaxed)) {
                        outcomes.extend(check(case, temp.path(), &store, &memo, &packages));
                    }
                    outcomes
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    suite.finish(outcomes);
}

/// The ratchet's outcomes for `case`, replayed in a copy of its project.
fn check(
    case: &lsp::Case,
    temp: &Path,
    store: &Store,
    memo: &Memo,
    packages: &[(String, String)],
) -> Vec<(String, Result<(), String>)> {
    let expected = suite_dir()
        .join("expected")
        .join(format!("{}.json", case.name));
    let expected: Value = serde_json::from_str(&fs::read_to_string(&expected).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", expected.display()));
    let cases = temp.join("cases");
    fs::create_dir_all(&cases).unwrap();
    let work = lsp::project_dir(&real(&cases), &case.name).unwrap();
    lsp::copy_project(case, &work).unwrap();
    let timing = Timing {
        quiet: Duration::from_millis(300),
        answer: Duration::from_secs(30),
        publishes: true,
    };
    let mut actual = replay(&work, &case.session, timing, store, memo, packages);
    // The transcripts were recorded on Linux; on Windows elm's report of a
    // misnamed file writes its paths with `\`.
    if cfg!(windows) {
        for diagnostics in
            (actual["diagnostics"].as_object_mut().into_iter()).flat_map(|d| d.values_mut())
        {
            for diagnostic in diagnostics.as_array_mut().into_iter().flatten() {
                if let Some(message) = diagnostic["message"].as_str()
                    && message.starts_with("UNEXPECTED FILE NAME")
                {
                    diagnostic["message"] = Value::String(message.replace('\\', "/"));
                }
            }
        }
    }
    // To compare by hand: nomi's transcripts, as the oracle's are written.
    if let Some(dir) = std::env::var_os("NOMI_LSP_DUMP") {
        let path = Path::new(&dir).join(format!("{}.json", case.name));
        fs::write(path, lsp::to_text(&actual)).unwrap();
    }
    lsp::outcomes(&case.name, &expected, &actual)
}
