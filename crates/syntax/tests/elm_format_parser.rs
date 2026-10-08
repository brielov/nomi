//! The port of elm-format's parser rejects exactly what elm-format
//! rejected in the mutation suite's oracle tables, at the position it
//! reported (plan 4.6a). It is where nomi reports a module it cannot
//! format from, so it is held to every case, not ratcheted.

mod common;

use nomi_base::ElmPosition;
use nomi_syntax::{Tool, elm_format};

#[test]
fn elm_format_parser_matches_elm_format() {
    let mismatches = common::each_seed(|seed| {
        let mut mismatches = Vec::new();
        for (m, cells) in &seed.cases {
            for ((_, tool), cell) in seed.columns.iter().zip(cells) {
                if *tool != Tool::ElmFormat || cell == "crash" {
                    continue;
                }
                let expected = match cell.split_once(':') {
                    None => Ok(()),
                    Some((row, column)) => Err(elm_format::Error::Position(ElmPosition {
                        row: row.parse().expect("a row"),
                        column: column.parse().expect("a column"),
                    })),
                };
                let verdict = elm_format::parse(&m.source);
                if verdict != expected {
                    mismatches.push(format!(
                        "{} / {}: elm-format {cell}, the port {verdict:?}",
                        seed.name, m.name
                    ));
                }
            }
        }
        mismatches
    });
    assert!(
        mismatches.is_empty(),
        "{} mismatches, the first:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
