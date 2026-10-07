//! `cargo xtask corpus format`: the corpus side of the format ratchet.
//! Each corpus project is a case. It passes when nomi formats each of its
//! modules to elm-format's output, and rejects the modules elm-format
//! rejects, as `corpus/format/oracle.tsv` records.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use nomi_testkit::ratchet::Suite;

use super::parallel;
use super::syntax::{self as corpus_syntax, CRASH};
use crate::download::sha256;

/// The suite's directory: the oracle's table and `passing.txt`.
pub const DIR: &str = "corpus/format";

/// What elm-format made of one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The module, unchanged.
    Same,
    /// Different text, with this hash.
    Sha256(String),
    /// Rejected, where it stopped parsing.
    Rejected(String),
    /// Crashed. nomi may format the module as it sees fit, a deviation
    /// `DECISIONS.md` records (plan 2.15).
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

/// The table's summary, and the outcome of each module that changes.
struct Table {
    modules: usize,
    sources: String,
    changed: BTreeMap<(String, String), Outcome>,
}

fn parse_table(text: &str) -> Result<Table> {
    let mut lines = text.lines().filter(|l| !l.starts_with("# @generated"));
    let summary = lines.next().context("the table is empty")?;
    let words: Vec<&str> = summary.split_whitespace().collect();
    let ["#", "modules", modules, "sources", sources] = words[..] else {
        bail!("malformed summary `{summary}`");
    };
    ensure!(
        lines.next() == Some("case\tpath\toutput"),
        "malformed column header"
    );
    let mut changed = BTreeMap::new();
    for line in lines {
        let [case, path, outcome] = line.split('\t').collect::<Vec<_>>()[..] else {
            bail!("malformed row `{line}`");
        };
        changed.insert((case.to_owned(), path.to_owned()), outcome.parse()?);
    }
    Ok(Table {
        modules: modules.parse()?,
        sources: sources.to_owned(),
        changed,
    })
}

/// Why nomi's result for `text` differs from `expected`, if it does.
fn mismatch(text: &[u8], expected: &Outcome) -> Option<String> {
    let formatted = std::str::from_utf8(text).ok().and_then(nomi_format::format);
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

pub fn check(root: &Path, record: bool) -> Result<()> {
    let dir = root.join(DIR);
    let text = std::fs::read_to_string(dir.join("oracle.tsv"))
        .context("reading corpus/format/oracle.tsv; run `cargo xtask oracle format corpus`")?;
    let table = parse_table(&text)?;
    let modules = corpus_syntax::modules(root)?;
    ensure!(
        table.modules == modules.len() && table.sources == corpus_syntax::digest(&modules)?,
        "the corpus changed since oracle.tsv was made; run `cargo xtask oracle format corpus`"
    );
    let found = parallel(&modules, |m| -> Result<Option<String>> {
        let text = std::fs::read(&m.file)?;
        let expected = table
            .changed
            .get(&(m.case.clone(), m.path.clone()))
            .unwrap_or(&Outcome::Same);
        Ok(mismatch(&text, expected).map(|why| format!("{}: {why}", m.path)))
    });
    let mut cases: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (m, found) in modules.iter().zip(found) {
        cases.entry(&m.case).or_default().extend(found?);
    }
    let outcomes = cases.into_iter().map(|(case, mismatches)| {
        let result = match mismatches.as_slice() {
            [] => Ok(()),
            [only] => Err(only.clone()),
            [first, rest @ ..] => Err(format!("{first} (and {} more)", rest.len())),
        };
        (case.to_owned(), result)
    });
    Suite::open(dir)
        .run_by("cargo xtask corpus format --record", record)
        .finish(outcomes);
    Ok(())
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
        assert_eq!((table.modules, table.sources.as_str()), (3, "abc"));
        let outcome = |case: &str, path: &str| &table.changed[&(case.to_owned(), path.to_owned())];
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
    fn mismatches_follow_the_outcome() {
        let formatted = b"module Main exposing (x)\n\n\nx =\n    1\n";
        assert_eq!(mismatch(formatted, &Outcome::Same), None);
        assert!(mismatch(formatted, &Outcome::Rejected("1:1".into())).is_some());
        assert_eq!(mismatch(formatted, &Outcome::Crashed), None);
        assert_eq!(mismatch(b"x =", &Outcome::Rejected("1:4".into())), None);
        assert!(mismatch(b"x =", &Outcome::Same).is_some());
    }
}
