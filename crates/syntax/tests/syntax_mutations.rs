//! The mutation side of the syntax ratchet: each mutation of each seed in
//! `syntax_mutations/seeds.txt` is a case, passing when nomi accepts and
//! rejects it as every tool in `syntax_mutations/oracle/<seed>.tsv` did.
//! `cargo xtask oracle syntax mutations` stores those tables.

use std::path::Path;

use nomi_base::Compat;
use nomi_syntax::{Source, Tool};
use nomi_testkit::mutation;
use nomi_testkit::ratchet::Suite;

const RERUN: &str = "rerun `cargo xtask oracle syntax mutations`";

#[test]
fn syntax_mutations() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/syntax_mutations");
    let seeds = std::fs::read_to_string(dir.join("seeds.txt")).expect("reading seeds.txt");
    let lines: Vec<&str> = seeds
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    // A thread per seed: the suite has tens of thousands of cases.
    let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
        let handles: Vec<_> = lines
            .iter()
            .map(|line| scope.spawn(|| seed_outcomes(&dir, line)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a seed's thread panicked"))
            .collect()
    });
    Suite::open(dir).finish(outcomes);
}

type Outcome = (String, Result<(), String>);

fn seed_outcomes(dir: &Path, line: &str) -> Vec<Outcome> {
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
    let columns: Vec<(&str, Tool)> = lines
        .next()
        .expect("no column header")
        .split('\t')
        .skip(1)
        .map(|column| match column {
            "elm-format" => (column, Tool::ElmFormat),
            _ => match column.strip_prefix("elm ").map(str::parse::<Compat>) {
                Some(Ok(compat)) => (column, Tool::Elm(source, compat)),
                _ => panic!("{seed}: unknown column `{column}`"),
            },
        })
        .collect();
    let rows: Vec<&str> = lines.collect();
    assert_eq!(rows.len(), mutations.len(), "{seed}: {RERUN}");
    let mut outcomes = Vec::with_capacity(rows.len());
    for (m, row) in mutations.iter().zip(rows) {
        let cells: Vec<&str> = row.split('\t').collect();
        assert_eq!(cells[0], m.name, "{seed}: {RERUN}");
        assert_eq!(
            cells.len(),
            columns.len() + 1,
            "{seed}: malformed row `{row}`"
        );
        let parse = nomi_syntax::parse_module(&m.source);
        // Releases rarely differ; validate each tool once.
        let mut verdicts: Vec<(Tool, bool)> = Vec::new();
        let mut accepts = |tool: Tool| match verdicts.iter().find(|(t, _)| *t == tool) {
            Some(&(_, accepted)) => accepted,
            None => {
                let accepted =
                    parse.errors().is_empty() && nomi_syntax::validate(&parse, tool).is_empty();
                verdicts.push((tool, accepted));
                accepted
            }
        };
        let mismatch = columns
            .iter()
            .zip(&cells[1..])
            .find_map(|(&(column, tool), &cell)| match (cell, accepts(tool)) {
                // A crash gives no verdict to match.
                ("crash", _) => None,
                ("ok", false) => Some(format!("{column} accepts, nomi rejects")),
                (_, true) if cell != "ok" => {
                    Some(format!("{column} rejects ({cell}), nomi accepts"))
                }
                _ => None,
            });
        outcomes.push((format!("{seed} / {}", m.name), mismatch.map_or(Ok(()), Err)));
    }
    outcomes
}
