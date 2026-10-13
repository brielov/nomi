//! The port of elm's parser rejects exactly what each elm rejected in the
//! mutation suite's oracle tables. It is what nomi reports from
//! when the grammar rejects a module, so it is held to every case, not
//! ratcheted.

mod common;
#[path = "common/mismatches.rs"]
mod mismatches;

use common::Verdict;
use nomi_syntax::{Tool, elm};

#[test]
fn elm_parser_matches_each_elm() {
    let mismatches = common::each_seed(|seed| {
        let mut mismatches = Vec::new();
        for (m, verdicts) in &seed.cases {
            for ((column, tool), verdict) in seed.columns.iter().zip(verdicts) {
                let Tool::Elm(source, compat) = *tool else {
                    continue;
                };
                if matches!(verdict, Verdict::Crashed) {
                    continue;
                }
                let port = elm::parse(m.source.as_bytes(), source, compat);
                if port.is_ok() != matches!(verdict, Verdict::Accepted) {
                    mismatches.push(format!(
                        "{} / {column}: elm {verdict}, the port {port:?}",
                        seed.case_id(m)
                    ));
                }
            }
        }
        mismatches
    });
    mismatches::assert_none(&mismatches);
}
