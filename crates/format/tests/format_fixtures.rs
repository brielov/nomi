//! The fixture side of the format ratchet: each file under
//! `format_fixtures/inputs/` is a case, passing when nomi formats it to
//! the file of the same name under `expected/`, or rejects it where
//! `rejected.tsv` says elm-format did, and keeping the formatter's
//! invariants but where `invariants.tsv` says elm-format breaks one.
//! `cargo xtask oracle format fixtures` stores the first two. Where nomi
//! deviates from elm-format on purpose, `deviations/` holds its output
//! instead, by hand, and the case keeps every invariant; each deviation
//! has its entry in `DECISIONS.md`.

use std::collections::BTreeMap;
use std::path::Path;

use nomi_format::{Invariant, invariant_mismatch};
use nomi_testkit::files_below;
use nomi_testkit::ratchet::{Expected, Suite};

const RERUN: &str = "rerun `cargo xtask oracle format fixtures`";

/// What nomi must make of a case.
enum Wanted {
    /// Nothing, as elm-format rejects it.
    Rejected(Rejection),
    /// Its own output, where it deviates from elm-format's on purpose.
    Deviation(Vec<u8>),
    /// elm-format's output.
    Formatted(Expected),
}

/// How elm-format rejects a case, as a cell of `rejected.tsv` says.
enum Rejection {
    /// It crashes, so nomi may do as it sees fit.
    Crash,
    /// It stops parsing at a `line:column`.
    At(String),
}

impl Rejection {
    fn parse(cell: &str) -> Rejection {
        match cell {
            "crash" => Rejection::Crash,
            at => Rejection::At(at.to_owned()),
        }
    }
}

#[test]
fn format_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/format_fixtures");
    let table = std::fs::read_to_string(dir.join("rejected.tsv"))
        .unwrap_or_else(|e| panic!("reading rejected.tsv: {e}; {RERUN}"));
    let rejected: BTreeMap<&str, &str> = rows(&table).collect();
    let table = std::fs::read_to_string(dir.join("invariants.tsv")).unwrap();
    // The case, and the invariants elm-format breaks there.
    let broken: BTreeMap<&str, Vec<Invariant>> = rows(&table)
        .map(|(case, cell)| {
            let invariants = cell.split(',').map(|name| {
                name.parse()
                    .unwrap_or_else(|e| panic!("invariants.tsv, {case}: {e}"))
            });
            (case, invariants.collect())
        })
        .collect();
    let suite = Suite::open(&dir);
    let cases = files_below(&dir.join("inputs")).unwrap();
    let deviations = files_below(&dir.join("deviations")).unwrap();
    for deviation in &deviations {
        assert!(
            cases.contains(deviation),
            "deviations/{deviation}: no input"
        );
    }
    let mut outcomes = Vec::new();
    for case in cases {
        let wanted = match rejected.get(case.as_str()) {
            Some(cell) => Wanted::Rejected(Rejection::parse(cell)),
            None if deviations.contains(&case) => {
                Wanted::Deviation(std::fs::read(dir.join("deviations").join(&case)).unwrap())
            }
            None => Wanted::Formatted(
                suite
                    .expected(&case)
                    .unwrap_or_else(|| panic!("{case}: no expected output; {RERUN}")),
            ),
        };
        let bytes = std::fs::read(dir.join("inputs").join(&case)).unwrap();
        let text = std::str::from_utf8(&bytes).ok();
        let formatted = text.and_then(nomi_format::format);
        let listed = broken.get(case.as_str()).map_or(&[][..], Vec::as_slice);
        let result = outcome(&wanted, formatted.as_deref()).and_then(|()| {
            invariant_mismatch(text.zip(formatted.as_deref()), listed).map_or(Ok(()), Err)
        });
        outcomes.push((case, result));
    }
    suite.finish(outcomes);
}

/// Why nomi's output, `formatted`, isn't what's `wanted`, if it isn't.
fn outcome(wanted: &Wanted, formatted: Option<&str>) -> Result<(), String> {
    match (wanted, formatted) {
        (Wanted::Rejected(Rejection::Crash), Some(_))
        | (Wanted::Rejected(Rejection::At(_)), None) => Ok(()),
        (Wanted::Rejected(Rejection::Crash), None) => {
            Err("elm-format crashes, nomi rejects".to_owned())
        }
        (Wanted::Rejected(Rejection::At(at)), Some(_)) => {
            Err(format!("elm-format rejects at {at}, nomi formats"))
        }
        (Wanted::Deviation(deviation), Some(output)) if output.as_bytes() == deviation => Ok(()),
        (Wanted::Deviation(_), _) => Err("nomi's output differs from its deviation".to_owned()),
        (Wanted::Formatted(_), None) => Err("elm-format formats, nomi rejects".to_owned()),
        (Wanted::Formatted(expected), Some(output)) if expected.matches(output.as_bytes()) => {
            Ok(())
        }
        (Wanted::Formatted(_), Some(_)) => {
            Err("nomi's output differs from elm-format's".to_owned())
        }
    }
}

/// A table's rows: a case, a tab, and a cell.
fn rows(table: &str) -> impl Iterator<Item = (&str, &str)> {
    table.lines().filter(|l| !l.starts_with('#')).map(|l| {
        l.split_once('\t')
            .unwrap_or_else(|| panic!("malformed row `{l}`"))
    })
}
