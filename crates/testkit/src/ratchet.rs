//! Parity ratchets (docs/plan.md, "Test tiers").
//!
//! A suite is a directory holding `passing.txt`, the sorted ids of cases
//! known to pass, and optionally the oracle's results for each case: either
//! a file under `expected/` or a line in `expected.sha256` for large ones.
//! A test runs every case and hands the outcomes to [`Suite::finish`]:
//!
//! - a listed case that fails, or is no longer run, is a regression and
//!   fails the test;
//! - a case that newly passes is reported, and recorded when the test runs
//!   under `cargo xtask ratchet <suite>`, which sets [`UPDATE_VAR`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// When set to `1`, [`Suite::finish`] adds newly passing cases to
/// `passing.txt`. It never removes a case.
pub const UPDATE_VAR: &str = "NOMI_RATCHET_UPDATE";

/// Regressions shown in a failure message before the rest are counted.
const SHOWN: usize = 50;

#[derive(Debug)]
pub struct Suite {
    name: String,
    dir: PathBuf,
    passing: BTreeSet<String>,
    /// Listed cases outside the run's scope, kept as they are.
    outside: BTreeSet<String>,
    hashes: BTreeMap<String, String>,
    update: bool,
    /// How to rerun the suite so it records newly passing cases.
    command: String,
}

/// What the oracle produced for one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    Bytes(Vec<u8>),
    Sha256(String),
}

/// The result of comparing a run with `passing.txt`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Listed cases that failed, with the reason.
    pub regressions: Vec<(String, String)>,
    /// Listed cases the run did not include.
    pub missing: Vec<String>,
    /// Passing cases that are not listed yet.
    pub newly_passing: Vec<String>,
    /// Cases run in total.
    pub run: usize,
    /// Cases that passed in total.
    pub passed: usize,
}

impl Suite {
    /// Opens the suite in `dir`, whose last component names it. Panics if
    /// `passing.txt` is missing, unsorted or has duplicates, so it stays a
    /// clean diff.
    pub fn open(dir: impl Into<PathBuf>) -> Suite {
        let dir = dir.into();
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = dir.join("passing.txt");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let passing = parse_passing(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let hashes = match std::fs::read_to_string(dir.join("expected.sha256")) {
            Ok(text) => parse_hashes(&text)
                .unwrap_or_else(|e| panic!("{}/expected.sha256: {e}", dir.display())),
            Err(_) => BTreeMap::new(),
        };
        Suite {
            dir,
            passing,
            outside: BTreeSet::new(),
            hashes,
            update: std::env::var(UPDATE_VAR).is_ok_and(|v| v == "1"),
            command: format!("cargo xtask ratchet {name}"),
            name,
        }
    }

    /// For a suite xtask runs itself rather than as a test target: `record`
    /// replaces [`UPDATE_VAR`], and `command` records newly passing cases.
    pub fn run_by(mut self, command: &str, record: bool) -> Suite {
        self.command = command.to_owned();
        self.update = record;
        self
    }

    /// Limits the run to the listed cases `keep` takes, as a run of some
    /// projects does: the others are neither missing nor dropped.
    pub fn within(mut self, keep: impl Fn(&str) -> bool) -> Suite {
        let (kept, outside) = self.passing.into_iter().partition(|case| keep(case));
        self.passing = kept;
        self.outside = outside;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The oracle's result for `case`, if the suite stores one.
    pub fn expected(&self, case: &str) -> Option<Expected> {
        if let Ok(bytes) = std::fs::read(self.dir.join("expected").join(case)) {
            return Some(Expected::Bytes(bytes));
        }
        self.hashes.get(case).cloned().map(Expected::Sha256)
    }

    /// Compares the run with `passing.txt`, records newly passing cases when
    /// asked to, and panics on regressions.
    pub fn finish(self, outcomes: impl IntoIterator<Item = (String, Result<(), String>)>) {
        let report = compare(&self.passing, outcomes);
        eprintln!(
            "ratchet {}: {}/{} passing, {} listed",
            self.name,
            report.passed,
            report.run,
            self.passing.len() + self.outside.len()
        );
        if !report.regressions.is_empty() || !report.missing.is_empty() {
            panic!("{}", failure_message(&self.name, &report));
        }
        if report.newly_passing.is_empty() {
            return;
        }
        if self.update {
            let mut passing = self.passing;
            passing.extend(self.outside);
            passing.extend(report.newly_passing.iter().cloned());
            let path = self.dir.join("passing.txt");
            std::fs::write(&path, render_passing(&passing))
                .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
            eprintln!(
                "ratchet {}: recorded {} newly passing",
                self.name,
                report.newly_passing.len()
            );
        } else {
            eprintln!(
                "ratchet {}: {} newly passing; record them with `{}`",
                self.name,
                report.newly_passing.len(),
                self.command
            );
        }
    }
}

impl Expected {
    pub fn matches(&self, actual: &[u8]) -> bool {
        match self {
            Expected::Bytes(bytes) => bytes == actual,
            Expected::Sha256(hash) => sha256_hex(actual) == *hash,
        }
    }
}

pub fn compare(
    passing: &BTreeSet<String>,
    outcomes: impl IntoIterator<Item = (String, Result<(), String>)>,
) -> Report {
    let mut report = Report::default();
    let mut seen = BTreeSet::new();
    for (case, result) in outcomes {
        report.run += 1;
        let listed = passing.contains(&case);
        match result {
            Ok(()) => {
                report.passed += 1;
                if !listed {
                    report.newly_passing.push(case.clone());
                }
            }
            Err(reason) if listed => report.regressions.push((case.clone(), reason)),
            Err(_) => {}
        }
        seen.insert(case);
    }
    report.missing = passing.difference(&seen).cloned().collect();
    report.regressions.sort();
    report.newly_passing.sort();
    report
}

fn failure_message(name: &str, report: &Report) -> String {
    let mut out = format!("ratchet {name}: ");
    if !report.regressions.is_empty() {
        let _ = writeln!(out, "{} listed case(s) now fail:", report.regressions.len());
        for (case, reason) in report.regressions.iter().take(SHOWN) {
            // The expectation and what was found, where a reason gives both.
            let mut lines = reason.lines();
            let _ = writeln!(out, "  {case}: {}", lines.next().unwrap_or(""));
            if let Some(found) = lines.next() {
                let _ = writeln!(out, "  {found}");
            }
        }
        if report.regressions.len() > SHOWN {
            let _ = writeln!(out, "  ... and {} more", report.regressions.len() - SHOWN);
        }
    }
    if !report.missing.is_empty() {
        let _ = writeln!(
            out,
            "{} listed case(s) were not run (remove them from passing.txt only with a reason in the commit):",
            report.missing.len()
        );
        for case in report.missing.iter().take(SHOWN) {
            let _ = writeln!(out, "  {case}");
        }
    }
    out
}

fn parse_passing(text: &str) -> Result<BTreeSet<String>, String> {
    let mut previous: Option<&str> = None;
    let mut cases = BTreeSet::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        if let Some(previous) = previous
            && line <= previous
        {
            return Err(format!(
                "{line:?} is out of order or repeated; keep it sorted"
            ));
        }
        cases.insert(line.to_string());
        previous = Some(line);
    }
    Ok(cases)
}

fn render_passing(cases: &BTreeSet<String>) -> String {
    cases.iter().map(|c| format!("{c}\n")).collect()
}

/// `sha256sum` format: `<hex>  <case>` per line.
fn parse_hashes(text: &str) -> Result<BTreeMap<String, String>, String> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (hash, case) = line
                .split_once("  ")
                .ok_or_else(|| format!("{line:?} is not `<sha256>  <case>`"))?;
            Ok((case.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

/// The SHA-256 of `bytes`, in lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn pass(case: &str) -> (String, Result<(), String>) {
        (case.to_string(), Ok(()))
    }

    fn fail(case: &str) -> (String, Result<(), String>) {
        (case.to_string(), Err(format!("{case} differs")))
    }

    #[test]
    fn regression_newly_passing_and_missing() {
        let report = compare(
            &set(&["a", "b", "gone"]),
            [pass("a"), fail("b"), pass("c"), fail("d")],
        );
        assert_eq!(report.regressions, [("b".into(), "b differs".into())]);
        assert_eq!(report.newly_passing, ["c"]);
        assert_eq!(report.missing, ["gone"]);
        assert_eq!((report.run, report.passed), (4, 2));
    }

    #[test]
    fn unsorted_passing_is_rejected() {
        assert!(parse_passing("a\nc\nb\n").is_err());
        assert!(parse_passing("a\na\n").is_err());
        assert_eq!(parse_passing("a\nb\n").unwrap(), set(&["a", "b"]));
    }

    #[test]
    fn expected_matches_bytes_or_hash() {
        assert!(Expected::Bytes(b"abc".to_vec()).matches(b"abc"));
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(Expected::Sha256(hash.into()).matches(b"abc"));
        assert!(!Expected::Sha256(hash.into()).matches(b"abd"));
    }

    #[test]
    fn expected_hashes_parse() {
        let hashes = parse_hashes("ABC  case-1\n").unwrap();
        assert_eq!(hashes["case-1"], "abc");
        assert!(parse_hashes("no-separator\n").is_err());
    }

    #[test]
    fn failure_message_lists_regressions() {
        let report = compare(&set(&["a"]), [fail("a")]);
        let message = failure_message("toy", &report);
        assert!(message.contains("1 listed case(s) now fail"));
        assert!(message.contains("  a: a differs"));
    }
}
