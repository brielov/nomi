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
mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_testkit::ratchet::Suite;
use nomi_testkit::transcript::Case;
use website::{Website, offline};

/// The elms the recordings ran, oldest first.
const RELEASES: [&str; 3] = ["0.19.1", "0.19.2", "0.19.3"];

/// The length of the recordings' project directory.
const ROOT_LENGTH: usize = 96;

#[test]
fn elm_make() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_make");
    let bin = tempfile::tempdir().unwrap();
    let elm = bin
        .path()
        .join(format!("elm{}", std::env::consts::EXE_SUFFIX));
    fs::copy(env!("CARGO_BIN_EXE_nomi"), &elm).unwrap();
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages");
    let cache = tempfile::tempdir().unwrap();
    Website::with_sources(&packages).fill(cache.path());
    let mut cases = Vec::new();
    for entry in fs::read_dir(dir.join("cases")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        cases.push((name, path));
    }
    cases.sort();
    assert!(cases.len() >= 600, "found only {} cases", cases.len());
    let mut runs = Vec::new();
    let mut crashes = 0;
    for (name, path) in &cases {
        let case = Case::parse(&fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = fs::read_to_string(dir.join("expected").join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle make`"));
        let terminal =
            fs::read_to_string(dir.join("expected_terminal").join(format!("{name}.txt")))
                .unwrap_or_else(|e| panic!("{name}: {e}; rerun `cargo xtask oracle make`"));
        let releases: &[&str] = if names_release(&case) {
            &RELEASES
        } else {
            &RELEASES[2..]
        };
        for release in releases {
            let id = format!("{name} as {release}");
            let Some(expected) = recorded(&expected, release) else {
                panic!("{id}: no transcript; rerun `cargo xtask oracle make`");
            };
            if crashed(expected) {
                crashes += 1;
                continue;
            }
            runs.push((id, as_release(&case, release), expected.to_owned()));
            let id = format!("{name} terminal as {release}");
            let Some(expected) = recorded(&terminal, release) else {
                panic!("{id}: no transcript; rerun `cargo xtask oracle make`");
            };
            if crashed(expected) {
                crashes += 1;
                continue;
            }
            let case = as_release(&terminal_case(&case), release);
            runs.push((id, case, steady(expected)));
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

/// Whether elm died on the case, by a signal or an internal error, where
/// nomi reports what it finds (DECISIONS.md, 2026-10-08).
fn crashed(expected: &str) -> bool {
    expected.starts_with("=== killed")
        || expected.contains("bypassed the normal error reporting process")
}

/// nomi's transcript of `case`, against `expected`.
fn run(elm: &Path, cache: &Path, case: &Case, expected: &str) -> Result<(), String> {
    let work = tempfile::tempdir().unwrap();
    let root = padded(work.path());
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
fn terminal_case(case: &Case) -> Case {
    let mut case = case.clone();
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
            out.push_str(&format!("{STEP}…/{n})"));
        } else {
            // Not a step after all: keep it as it is.
            out.push_str(STEP);
            rest = &rest[STEP.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// Whether the case is an application whose `elm-version` names a
/// pinned elm.
fn names_release(case: &Case) -> bool {
    (case.files.iter()).any(|(path, text)| {
        path == "elm.json"
            && text.contains("\"type\": \"application\"")
            && (RELEASES.iter()).any(|r| text.contains(&format!("\"elm-version\": \"{r}\"")))
    })
}

/// The case with its application's pinned `elm-version` set to
/// `release`, as the recordings ran it.
fn as_release(case: &Case, release: &str) -> Case {
    let mut case = case.clone();
    for (path, text) in &mut case.files {
        if path != "elm.json" || !text.contains("\"type\": \"application\"") {
            continue;
        }
        for pinned in RELEASES {
            let from = format!("\"elm-version\": \"{pinned}\"");
            *text = text.replace(&from, &format!("\"elm-version\": \"{release}\""));
        }
    }
    case
}

/// The transcript `expected` records for elm `release`.
fn recorded<'a>(expected: &'a str, release: &str) -> Option<&'a str> {
    let elm = format!("elm {release}");
    let mut groups = expected.split("### ").skip(1);
    groups.find_map(|group| {
        let (elms, transcript) = group.split_once('\n')?;
        elms.split(", ").any(|e| e == elm).then_some(transcript)
    })
}

/// A new directory in `base` whose canonical path is [`ROOT_LENGTH`]
/// bytes long.
fn padded(base: &Path) -> PathBuf {
    let base = fs::canonicalize(base).unwrap();
    let pad = ROOT_LENGTH - base.as_os_str().len() - 1;
    let dir = base.join("p".repeat(pad));
    fs::create_dir_all(&dir).unwrap();
    dir
}
