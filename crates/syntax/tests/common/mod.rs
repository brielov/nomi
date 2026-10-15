//! The mutation suite's seeds and oracle tables, which more than one test
//! reads.

use std::fmt;
use std::path::{Path, PathBuf};

use nomi_syntax::{Source, Tool};
use nomi_testkit::mutation::{self, Mutation};
use nomi_testkit::release_column;

pub(crate) const RERUN: &str = "rerun `cargo xtask oracle syntax mutations`";

/// `tests/<name>`.
pub(crate) fn tests_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

/// `tests/syntax_mutations`.
pub(crate) fn dir() -> PathBuf {
    tests_dir("syntax_mutations")
}

/// A seed's mutations, each with what every tool in its table did.
pub(crate) struct Seed {
    pub name: String,
    /// The table's columns, by header and tool.
    pub columns: Vec<(String, Tool)>,
    /// Each mutation and its verdicts, one per column.
    pub cases: Vec<(Mutation, Vec<Verdict>)>,
}

impl Seed {
    /// How a ratchet names the case of `mutation`.
    pub(crate) fn case_id(&self, mutation: &Mutation) -> String {
        format!("{} / {}", self.name, mutation.name)
    }
}

/// What a tool did with a mutation, as a cell of the oracle table says.
pub(crate) enum Verdict {
    /// `ok`.
    Accepted,
    /// `crash`, which gives no verdict to match.
    Crashed,
    /// What it reported: elm's title and region, or elm-format's
    /// `row:col`.
    Rejected(String),
}

impl Verdict {
    fn parse(cell: &str) -> Verdict {
        match cell {
            "ok" => Verdict::Accepted,
            "crash" => Verdict::Crashed,
            _ => Verdict::Rejected(cell.to_owned()),
        }
    }
}

impl fmt::Display for Verdict {
    /// The cell, as the table has it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Accepted => f.write_str("ok"),
            Verdict::Crashed => f.write_str("crash"),
            Verdict::Rejected(cell) => f.write_str(cell),
        }
    }
}

/// The seeds of `seeds.txt`, run a thread each: the suite has tens of
/// thousands of cases.
pub(crate) fn each_seed<T: Send>(run: impl Fn(Seed) -> Vec<T> + Sync) -> Vec<T> {
    let dir = dir();
    let seeds = mutation::seeds::<Source>(&dir).unwrap_or_else(|e| panic!("{e}"));
    std::thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .iter()
            .map(|seed| scope.spawn(|| run(cases(&dir, seed))))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a seed's thread panicked"))
            .collect()
    })
}

/// `seed`'s mutations, with the verdicts its oracle table keeps.
fn cases(dir: &Path, seed: &mutation::Seed<Source>) -> Seed {
    let name = &seed.name;
    let mutations = mutation::suite(name, &seed.text);
    let table = std::fs::read_to_string(dir.join("oracle").join(format!("{name}.tsv")))
        .unwrap_or_else(|e| panic!("reading oracle/{name}.tsv: {e}; {RERUN}"));
    let mut lines = table.lines().filter(|l| !l.starts_with("# @generated"));
    let digest = lines.next().and_then(|l| l.strip_prefix("# sources "));
    assert_eq!(
        digest,
        Some(mutation::digest(&mutations).as_str()),
        "{name}: the seed or the generator changed; {RERUN}"
    );
    let columns: Vec<(String, Tool)> = lines
        .next()
        .expect("no column header")
        .split('\t')
        .skip(1)
        .map(|column| match column {
            "elm-format" => (column.to_owned(), Tool::ElmFormat),
            _ => match release_column(column) {
                Some(compat) => (column.to_owned(), Tool::Elm(seed.kind, compat)),
                None => panic!("{name}: unknown column `{column}`"),
            },
        })
        .collect();
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), mutations.len(), "{name}: {RERUN}");
    let cases = mutations
        .into_iter()
        .zip(rows)
        .map(|(m, row)| {
            let cells: Vec<&str> = row.split('\t').collect();
            assert_eq!(cells[0], m.name, "{name}: {RERUN}");
            assert_eq!(
                cells.len(),
                columns.len() + 1,
                "{name}: malformed row `{row}`"
            );
            (m, cells[1..].iter().copied().map(Verdict::parse).collect())
        })
        .collect();
    Seed {
        name: name.clone(),
        columns,
        cases,
    }
}
