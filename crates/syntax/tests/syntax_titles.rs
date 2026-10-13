//! The first syntax report ratchet: each mutation some elm rejects is a
//! case, passing when the port of elm's reports titles and places the
//! error as every such elm did, in `syntax_mutations/oracle/<seed>.tsv`.

mod common;
#[path = "common/elm_reports.rs"]
mod elm_reports;

use std::path::Path;

use nomi_testkit::ratchet::Suite;

#[test]
fn syntax_titles() {
    let outcomes = common::each_seed(|seed| {
        elm_reports::rejections(&seed)
            .into_iter()
            .map(|(case, rejections)| {
                let outcome = rejections.into_iter().try_for_each(|r| {
                    let problem = r.problem.map_err(|e| format!("{}: {e}", r.column))?;
                    let shift = i64::from(r.compat.first_json_position())
                        - i64::from(r.compat.first_position());
                    let at = |p: nomi_base::diagnostic::code::Position| {
                        format!("{}:{}", i64::from(p.row) + shift, i64::from(p.col) + shift)
                    };
                    let (start, end) = (problem.region.start, problem.region.end);
                    let got = format!("{} {}-{}", problem.title, at(start), at(end));
                    if got == r.cell {
                        Ok(())
                    } else {
                        Err(format!("{}: elm {}, the port {got}", r.column, r.cell))
                    }
                });
                (case, outcome)
            })
            .collect()
    });
    Suite::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/syntax_titles")).finish(outcomes);
}
