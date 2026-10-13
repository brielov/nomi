//! The port of elm's parser rejects exactly what each elm rejected in the
//! mutation suite's oracle tables. It is what nomi reports from
//! when the grammar rejects a module, so it is held to every case, not
//! ratcheted.

mod common;

use nomi_syntax::{Tool, elm};

#[test]
fn elm_parser_matches_each_elm() {
    let mismatches = common::each_seed(|seed| {
        let mut mismatches = Vec::new();
        for (m, cells) in &seed.cases {
            for ((column, tool), cell) in seed.columns.iter().zip(cells) {
                let Tool::Elm(source, compat) = *tool else {
                    continue;
                };
                if cell == "crash" {
                    continue;
                }
                let verdict = elm::parse(m.source.as_bytes(), source, compat);
                if verdict.is_ok() != (cell == "ok") {
                    mismatches.push(format!(
                        "{} / {} / {column}: elm {cell}, the port {verdict:?}",
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
