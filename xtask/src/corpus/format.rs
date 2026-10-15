//! `cargo xtask corpus format`: the corpus side of the format ratchet.
//! Each corpus project is a case. It passes when nomi formats each of its
//! modules to elm-format's output, and rejects the modules elm-format
//! rejects, as `corpus/format/oracle.tsv` records, and keeps the
//! formatter's invariants but where `corpus/format/invariants.tsv` says
//! elm-format breaks one. Where nomi deviates from elm-format on purpose,
//! `corpus/format/deviations.tsv` holds the hash of nomi's output
//! instead, with the decision it follows.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use nomi_format::{Invariant, invariant_mismatch};
use nomi_testkit::sha256_hex as sha256;

use super::syntax::{self as corpus_syntax, CRASH, Sources};
use crate::oracle::cores;
use nomi_testkit::parallel_on;

/// The suite's directory: the oracle's table and `passing.txt`.
pub(crate) const DIR: &str = "corpus/format";

/// What elm-format made of one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The module, unchanged.
    Same,
    /// Different text, with this hash.
    Sha256(String),
    /// Rejected, where it stopped parsing.
    Rejected(String),
    /// Crashed. nomi may format the module as it sees fit, a deviation
    /// `DECISIONS.md` records.
    Crashed,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Same => f.write_str("same"),
            Outcome::Sha256(hash) => f.write_str(hash),
            Outcome::Rejected(at) => write!(f, "rejected {at}"),
            Outcome::Crashed => f.write_str(CRASH),
        }
    }
}

impl std::str::FromStr for Outcome {
    type Err = anyhow::Error;

    fn from_str(cell: &str) -> Result<Outcome> {
        Ok(match cell {
            "same" => Outcome::Same,
            CRASH => Outcome::Crashed,
            _ => match cell.strip_prefix("rejected ") {
                Some(at) => Outcome::Rejected(at.to_owned()),
                None if cell.len() == 64 && cell.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    Outcome::Sha256(cell.to_owned())
                }
                None => bail!("malformed outcome `{cell}`"),
            },
        })
    }
}

/// The table: what it was made from, and the outcome of each module
/// that changes, by case and path.
struct Table<'a> {
    sources: Sources,
    changed: BTreeMap<(&'a str, &'a str), Outcome>,
}

/// The rows of a table after its comments and its `header`, each of `N`
/// cells, by their first two: the case and path.
fn rows<'a, const N: usize>(
    text: &'a str,
    header: &str,
) -> Result<BTreeMap<(&'a str, &'a str), [&'a str; N]>> {
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    ensure!(lines.next() == Some(header), "malformed column header");
    let mut rows = BTreeMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let row: [&str; N] = (cells.try_into()).map_err(|_| anyhow!("malformed row `{line}`"))?;
        let fresh = rows.insert((row[0], row[1]), row).is_none();
        ensure!(fresh, "{} {} is in the table twice", row[0], row[1]);
    }
    Ok(rows)
}

fn parse_table(text: &str) -> Result<Table<'_>> {
    let summary = (text.lines())
        .find(|l| !l.starts_with("# @generated"))
        .context("the table is empty")?;
    let changed = (rows(text, "case\tpath\toutput")?.into_iter())
        .map(|(key, [_, _, outcome])| Ok((key, outcome.parse()?)))
        .collect::<Result<_>>()?;
    Ok(Table {
        sources: summary.parse()?,
        changed,
    })
}

/// Why nomi's result for `text`, `formatted`, differs from `expected`, if
/// it does.
fn mismatch(text: &[u8], formatted: Option<&str>, expected: &Outcome) -> Option<String> {
    match (expected, formatted) {
        (Outcome::Rejected(_), None) | (Outcome::Crashed, Some(_)) => None,
        (Outcome::Rejected(at), Some(_)) => {
            Some(format!("elm-format rejects at {at}, nomi formats"))
        }
        (_, None) => Some("elm-format formats, nomi rejects".to_owned()),
        (Outcome::Same, Some(output)) => {
            (output.as_bytes() != text).then(|| "nomi changes a formatted module".to_owned())
        }
        (Outcome::Sha256(hash), Some(output)) => (sha256(output.as_bytes()) != *hash)
            .then(|| "nomi's output differs from elm-format's".to_owned()),
    }
}

/// The invariants elm-format breaks in each module where it breaks one,
/// by case and path.
fn parse_invariants(text: &str) -> Result<BTreeMap<(&str, &str), Vec<Invariant>>> {
    (rows(text, "case\tpath\tinvariant")?.into_iter())
        .map(|(key, [_, _, invariants])| {
            let invariants = invariants
                .split(',')
                .map(str::parse)
                .collect::<Result<_, _>>()?;
            Ok((key, invariants))
        })
        .collect()
}

/// nomi's output where it deviates, by case and path.
fn parse_deviations(text: &str) -> Result<BTreeMap<(&str, &str), Outcome>> {
    (rows(text, "case\tpath\toutput\twhy")?.into_iter())
        .map(|(key, [_, _, output, _why])| {
            let output = output.parse()?;
            ensure!(
                matches!(output, Outcome::Sha256(_)),
                "a deviation is an output's hash: `{output}`"
            );
            Ok((key, output))
        })
        .collect()
}

pub(crate) fn check(root: &Path, record: bool) -> Result<()> {
    let dir = root.join(DIR);
    let read = |file: &str| {
        fs::read_to_string(dir.join(file)).with_context(|| format!("reading {DIR}/{file}"))
    };
    let deviations = read("deviations.tsv")?;
    let deviations = parse_deviations(&deviations)?;
    let invariants = read("invariants.tsv")?;
    let broken = parse_invariants(&invariants)?;
    let text = read("oracle.tsv").context("run `cargo xtask oracle format corpus`")?;
    let table = parse_table(&text)?;
    let modules = corpus_syntax::modules(root)?;
    table.sources.ensure_current(&modules, "format")?;
    for (table, (case, path)) in (broken.keys().map(|k| ("invariants.tsv", k)))
        .chain(deviations.keys().map(|k| ("deviations.tsv", k)))
    {
        ensure!(
            modules.iter().any(|m| m.case == *case && m.path == *path),
            "{table} lists {case} {path}, which the corpus lacks"
        );
    }
    let found = parallel_on(cores(), &modules, |m| -> Result<Option<String>> {
        let text = fs::read(&m.file)?;
        let key = (m.case.as_str(), m.path.as_str());
        let expected = (deviations.get(&key))
            .or_else(|| table.changed.get(&key))
            .unwrap_or(&Outcome::Same);
        let listed = broken.get(&key).map_or(&[][..], Vec::as_slice);
        let formatted = std::str::from_utf8(&text)
            .ok()
            .and_then(nomi_format::format);
        let formatted = formatted.as_deref();
        let why = mismatch(&text, formatted, expected)
            .or_else(|| invariant_mismatch(&text, formatted, listed));
        Ok(why.map(|why| format!("{}: {why}", m.path)))
    });
    corpus_syntax::finish(
        &dir,
        "cargo xtask corpus format --record",
        record,
        &modules,
        found,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_parse() {
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let text = format!(
            "# @generated by `cargo xtask oracle format`; regenerate, never edit.\n\
             # modules 3 sources abc\n\
             case\tpath\toutput\n\
             a/b@1.0.0\tsrc/A.elm\t{hash}\n\
             a/b@1.0.0\tsrc/B.elm\trejected 1:2\n\
             app/c\tsrc/C.elm\tcrash\n"
        );
        let table = parse_table(&text).unwrap();
        assert_eq!(table.sources, "# modules 3 sources abc".parse().unwrap());
        let outcome = |case: &'static str, path: &'static str| &table.changed[&(case, path)];
        assert_eq!(
            outcome("a/b@1.0.0", "src/A.elm"),
            &Outcome::Sha256(hash.into())
        );
        assert_eq!(
            outcome("a/b@1.0.0", "src/B.elm"),
            &Outcome::Rejected("1:2".into())
        );
        assert_eq!(outcome("app/c", "src/C.elm"), &Outcome::Crashed);
        assert!("nonsense".parse::<Outcome>().is_err());
    }

    #[test]
    fn outcomes_round_trip() {
        for outcome in [
            Outcome::Same,
            Outcome::Sha256("a".repeat(64)),
            Outcome::Rejected("3:4".into()),
            Outcome::Crashed,
        ] {
            assert_eq!(outcome.to_string().parse::<Outcome>().unwrap(), outcome);
        }
    }

    #[test]
    fn invariant_tables_parse() {
        let text = "# comment\ncase\tpath\tinvariant\na/b@1.0.0\tsrc/A.elm\tparses,idempotent\n";
        let broken = parse_invariants(text).unwrap();
        assert_eq!(
            broken[&("a/b@1.0.0", "src/A.elm")],
            [Invariant::Parses, Invariant::Idempotent]
        );
        assert!(parse_invariants("case\tpath\tinvariant\na\tb\tnonsense\n").is_err());
        assert!(parse_invariants("a\tb\tidempotent\n").is_err());
    }

    #[test]
    fn mismatches_follow_the_outcome() {
        let mismatch = |text: &[u8], expected: &Outcome| {
            let formatted = std::str::from_utf8(text).ok().and_then(nomi_format::format);
            mismatch(text, formatted.as_deref(), expected)
        };
        let formatted = b"module Main exposing (x)\n\n\nx =\n    1\n";
        assert_eq!(mismatch(formatted, &Outcome::Same), None);
        assert!(mismatch(formatted, &Outcome::Rejected("1:1".into())).is_some());
        assert_eq!(mismatch(formatted, &Outcome::Crashed), None);
        assert_eq!(mismatch(b"x =", &Outcome::Rejected("1:4".into())), None);
        assert!(mismatch(b"x =", &Outcome::Same).is_some());
    }
}
