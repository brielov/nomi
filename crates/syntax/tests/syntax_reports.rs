//! The second syntax report ratchet: each mutation some elm rejects is a
//! case, passing when the port of elm's reports writes every such elm's
//! `--report=json` byte for byte, as hashed in
//! `syntax_mutations/reports/<seed>.tsv`.

mod common;
#[path = "common/elm_reports.rs"]
mod elm_reports;

use std::collections::HashMap;
use std::path::Path;

use nomi_base::diagnostic::json;
use nomi_testkit::ratchet::{Suite, sha256_hex};

/// The length of the hash prefixes the table keeps.
const HASH: usize = 16;

#[test]
fn syntax_reports() {
    let outcomes = common::each_seed(|seed| {
        let hashes = hashes(&seed.name);
        elm_reports::rejections(&seed)
            .into_iter()
            .map(|(case, rejections)| {
                let name = case.split_once(" / ").map_or("", |(_, m)| m);
                let outcome = rejections.into_iter().try_for_each(|r| {
                    let problem = r.problem.map_err(|e| format!("{}: {e}", r.column))?;
                    let expected = hashes
                        .get(&(name.to_owned(), r.column.clone()))
                        .ok_or_else(|| format!("{}: no hash; {}", r.column, common::RERUN))?;
                    let report = json::compile_errors(vec![json::module(
                        "A.elm",
                        "???",
                        vec![problem.to_json(r.compat)],
                    )])
                    .to_string();
                    let got = &sha256_hex(report.as_bytes())[..HASH];
                    if got == expected {
                        Ok(())
                    } else {
                        Err(format!("{}: elm's report differs ({})", r.column, r.cell))
                    }
                });
                (case, outcome)
            })
            .collect()
    });
    Suite::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/syntax_reports"))
        .finish(outcomes);
}

/// `reports/<seed>.tsv`, by mutation and column.
fn hashes(seed: &str) -> HashMap<(String, String), String> {
    let path = common::dir().join("reports").join(format!("{seed}.tsv"));
    let table = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}; {}", path.display(), common::RERUN));
    let mut lines = table.lines().filter(|l| !l.starts_with('#'));
    let columns: Vec<&str> = lines.next().expect("no header").split('\t').collect();
    let mut hashes = HashMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        for (column, cell) in columns.iter().zip(&cells).skip(1) {
            hashes.insert(
                (cells[0].to_owned(), (*column).to_owned()),
                (*cell).to_owned(),
            );
        }
    }
    hashes
}
