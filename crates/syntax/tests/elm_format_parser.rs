//! The port of elm-format's parser rejects exactly what elm-format
//! rejected in the mutation suite's oracle tables, at the position it
//! reported. It is where nomi reports a module it cannot
//! format from, so it is held to every case, not ratcheted.

mod common;
#[path = "common/mismatches.rs"]
mod mismatches;

use common::Verdict;
use nomi_base::ElmPosition;
use nomi_syntax::{Tool, elm_format};

#[test]
fn elm_format_parser_matches_elm_format() {
    let mismatches = common::each_seed(|seed| {
        let mut mismatches = Vec::new();
        for (m, verdicts) in &seed.cases {
            for ((_, tool), verdict) in seed.columns.iter().zip(verdicts) {
                let expected = match verdict {
                    _ if *tool != Tool::ElmFormat => continue,
                    Verdict::Crashed => continue,
                    Verdict::Accepted => Ok(()),
                    Verdict::Rejected(cell) => {
                        let (row, column) = cell.split_once(':').expect("a row and a column");
                        Err(elm_format::Error::Position(ElmPosition {
                            row: row.parse().expect("a row"),
                            column: column.parse().expect("a column"),
                        }))
                    }
                };
                let port = elm_format::parse(&m.source);
                if port != expected {
                    mismatches.push(format!(
                        "{}: elm-format {verdict}, the port {port:?}",
                        seed.case_id(m)
                    ));
                }
            }
        }
        mismatches
    });
    mismatches::assert_none(&mismatches);
}
