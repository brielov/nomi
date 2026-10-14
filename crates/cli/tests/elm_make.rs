//! The CLI report matrix ratchet: nomi invoked as `elm` on
//! each case of `elm_make/cases/`, passing when its transcript is the one
//! `cargo xtask oracle make` recorded from the elm it emulates. An
//! application whose `elm-version` names a pinned elm runs as each, with
//! its `elm-version` rewritten; any other case as the newest, which is
//! how nomi builds a package.
//!
//! Cases run offline, with a cache serving the packages vendored for the
//! compiler's tests, in a directory padded to the recordings' length, as
//! messages wrap by the paths they name. Where elm crashed, there is
//! nothing to match, and the run is skipped.
//!
//! Each case runs again without `--report=json`, as `<case> terminal`,
//! against `expected_terminal/`. How many `Verifying
//! dependencies (k/n)` steps elm prints depends on how its threads
//! interleave, and varies between runs of one elm, so a run of them reads
//! as one step to `n` on both sides (DECISIONS.md, 2026-10-11).
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_base::Compat;
use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::{self, Case, ROOT_LENGTH, crashed, recorded};
use website::offline;

#[test]
fn elm_make() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_make");
    let (_bin, elm) = website::personality("elm");
    let cache = website::fixture_cache();
    let cases = transcript::cases(&dir.join("cases"));
    assert!(cases.len() >= 600, "found only {} cases", cases.len());
    let mut runs = Vec::new();
    let mut crashes = 0;
    for (name, case) in &cases {
        let expected = fs::read_to_string(dir.join("expected").join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle make`"));
        let expected_terminal =
            fs::read_to_string(dir.join("expected_terminal").join(format!("{name}.txt")))
                .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle make`"));
        let releases: &[Compat] = if case.pins_release() {
            &Compat::ALL
        } else {
            &[Compat::LATEST]
        };
        for &release in releases {
            let id = format!("{name} as {release}");
            let Some(expected) = recorded(&expected, release) else {
                panic!("{id}: no transcript; rerun `cargo xtask oracle make`");
            };
            if crashed(expected) {
                crashes += 1;
                continue;
            }
            runs.push((id, case.as_release(release), expected.to_owned()));
            let id = format!("{name} terminal as {release}");
            let Some(expected) = recorded(&expected_terminal, release) else {
                panic!("{id}: no transcript; rerun `cargo xtask oracle make`");
            };
            if crashed(expected) {
                crashes += 1;
                continue;
            }
            runs.push((id, terminal(case.as_release(release)), steady(expected)));
        }
    }
    // Each run is a process; a few at once.
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let next = AtomicUsize::new(0);
    let outcomes: Vec<(String, Result<(), String>)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((id, case, expected)) = runs.get(i) else {
                            return done;
                        };
                        let outcome = run(&elm, cache.path(), case, expected);
                        if let (Err(why), Ok(_)) = (&outcome, std::env::var("NOMI_SHOW")) {
                            eprintln!("FAIL {id}\n{why}");
                        }
                        done.push((i, (id.clone(), outcome)));
                    }
                })
            })
            .collect();
        let mut all: Vec<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        all.sort_by_key(|(i, _)| *i);
        all.into_iter().map(|(_, outcome)| outcome).collect()
    });
    eprintln!("elm_make: {crashes} runs skipped where elm crashed");
    Suite::open(dir).finish(outcomes);
}

/// nomi's transcript of `case`, against `expected`.
fn run(elm: &Path, cache: &Path, case: &Case, expected: &str) -> Result<(), String> {
    let work = tempfile::tempdir().unwrap();
    let root =
        nomi_testkit::padded_dir(&fs::canonicalize(work.path()).unwrap(), ROOT_LENGTH).unwrap();
    case.create_files(&root).unwrap();
    case.date_files(&root).unwrap();
    let actual = case
        .run(offline(elm, cache), &root)
        .unwrap()
        .compared(case, &root);
    let actual = steady(&actual);
    if actual == expected {
        Ok(())
    } else {
        Err(format!("--- expected\n{expected}--- actual\n{actual}"))
    }
}

/// `case` in the terminal style: without `--report=json`.
fn terminal(mut case: Case) -> Case {
    case.args.retain(|a| a != "--report=json");
    case
}

/// `transcript` with each run of `\rVerifying dependencies (k/n)` steps
/// read as the one step `\rVerifying dependencies (…/n)`.
fn steady(transcript: &str) -> String {
    const STEP: &str = "\rVerifying dependencies (";
    let mut out = String::new();
    let mut rest = transcript;
    while let Some(at) = rest.find(STEP) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let mut total = None;
        while let Some(step) = rest.strip_prefix(STEP)
            && let Some(close) = step.find(')')
            && let Some((_, n)) = step[..close].split_once('/')
        {
            total = Some(n);
            rest = &step[close + 1..];
        }
        if let Some(n) = total {
            write!(out, "{STEP}…/{n})").unwrap();
        } else {
            // Not a step after all: keep it as it is.
            out.push_str(STEP);
            rest = &rest[STEP.len()..];
        }
    }
    out.push_str(rest);
    out
}
