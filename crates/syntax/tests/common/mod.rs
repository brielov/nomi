//! The mutation suite's seeds and oracle tables, which more than one test
//! reads.

use std::path::{Path, PathBuf};

use nomi_base::Compat;
use nomi_syntax::{Source, Tool};
use nomi_testkit::mutation::{self, Mutation};

pub const RERUN: &str = "rerun `cargo xtask oracle syntax mutations`";

/// `tests/syntax_mutations`.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/syntax_mutations")
}

/// A seed's mutations, each with what every tool in its table did.
pub struct Seed {
    pub name: String,
    /// The table's columns, by header and tool.
    pub columns: Vec<(String, Tool)>,
    /// Each mutation and its cells, one per column.
    pub cases: Vec<(Mutation, Vec<String>)>,
}

/// The lines of `seeds.txt`, run a thread each: the suite has tens of
/// thousands of cases.
pub fn each_seed<T: Send>(run: impl Fn(Seed) -> Vec<T> + Sync) -> Vec<T> {
    let dir = dir();
    let seeds = std::fs::read_to_string(dir.join("seeds.txt")).expect("reading seeds.txt");
    let lines: Vec<&str> = seeds
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = lines
            .iter()
            .map(|line| scope.spawn(|| run(seed(&dir, line))))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a seed's thread panicked"))
            .collect()
    })
}

fn seed(dir: &Path, line: &str) -> Seed {
    let words: Vec<&str> = line.split_whitespace().collect();
    let [seed, kind, file] = words[..] else {
        panic!("seeds.txt: malformed line `{line}`");
    };
    let source = match kind {
        "application" => Source::Application,
        "package" => Source::Package,
        "kernel" => Source::Kernel,
        _ => panic!("seeds.txt: unknown kind `{kind}`"),
    };
    let text = std::fs::read_to_string(dir.join("seeds").join(file))
        .unwrap_or_else(|e| panic!("reading {file}: {e}"));
    let mutations = mutation::suite(seed, &text);
    let table = std::fs::read_to_string(dir.join("oracle").join(format!("{seed}.tsv")))
        .unwrap_or_else(|e| panic!("reading oracle/{seed}.tsv: {e}; {RERUN}"));
    let mut lines = table.lines().filter(|l| !l.starts_with("# @generated"));
    let digest = lines.next().and_then(|l| l.strip_prefix("# sources "));
    assert_eq!(
        digest,
        Some(mutation::digest(&mutations).as_str()),
        "{seed}: the seed or the generator changed; {RERUN}"
    );
    let columns: Vec<(String, Tool)> = lines
        .next()
        .expect("no column header")
        .split('\t')
        .skip(1)
        .map(|column| match column {
            "elm-format" => (column.to_owned(), Tool::ElmFormat),
            _ => match column.strip_prefix("elm ").map(str::parse::<Compat>) {
                Some(Ok(compat)) => (column.to_owned(), Tool::Elm(source, compat)),
                _ => panic!("{seed}: unknown column `{column}`"),
            },
        })
        .collect();
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), mutations.len(), "{seed}: {RERUN}");
    let cases = mutations
        .into_iter()
        .zip(rows)
        .map(|(m, row)| {
            let cells: Vec<String> = row.split('\t').map(str::to_owned).collect();
            assert_eq!(cells[0], m.name, "{seed}: {RERUN}");
            assert_eq!(
                cells.len(),
                columns.len() + 1,
                "{seed}: malformed row `{row}`"
            );
            (m, cells[1..].to_vec())
        })
        .collect();
    Seed {
        name: seed.to_owned(),
        columns,
        cases,
    }
}
