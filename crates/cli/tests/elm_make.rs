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

use nomi_testkit::parallel_on;
use nomi_testkit::ratchet::{Suite, shown};
use nomi_testkit::transcript::{self, Case, crashed, recorded};
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
        let read = |expected: &str| {
            fs::read_to_string(dir.join(expected).join(format!("{name}.txt")))
                .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle make`"))
        };
        let (json, terminal_style) = (read("expected"), read("expected_terminal"));
        for &release in case.releases() {
            let case = case.clone().as_release(release);
            let styles = [
                ("", &json, case.clone()),
                (" terminal", &terminal_style, terminal(case)),
            ];
            for (style, expected, case) in styles {
                let id = format!("{name}{style} as {release}");
                let Some(expected) = recorded(expected, release) else {
                    panic!("{id}: no transcript; rerun `cargo xtask oracle make`");
                };
                // Where elm crashed in JSON, it isn't run in the terminal
                // style either.
                if crashed(expected) {
                    crashes += 1;
                    break;
                }
                runs.push(Run {
                    id,
                    case,
                    expected: steady(expected),
                });
            }
        }
    }
    // Each run is a process; a few at once.
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let outcomes = parallel_on(workers, &runs, |Run { id, case, expected }| {
        let actual = website::transcript(case, offline(&elm, cache.path()));
        shown(id.clone(), transcript::matching(expected, &steady(&actual)))
    });
    eprintln!("elm_make: {crashes} runs skipped where elm crashed");
    Suite::open(dir).finish(outcomes);
}

/// One replay of a case: its id, the case as run, and the transcript
/// elm gave.
struct Run {
    id: String,
    case: Case,
    expected: String,
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
