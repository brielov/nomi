//! The first syntax report ratchet: each mutation some elm rejects is a
//! case, passing when the port of elm's reports titles and places the
//! error as every such elm did, in `syntax_mutations/oracle/<seed>.tsv`.

mod common;
#[path = "common/elm_reports.rs"]
mod elm_reports;

use nomi_testkit::ratchet::Suite;

#[test]
fn syntax_titles() {
    let outcomes = common::each_seed(|seed| {
        elm_reports::rejections(&seed)
            .into_iter()
            .map(|case| {
                let outcome = case.rejections.into_iter().try_for_each(|r| {
                    let problem =
                        (r.problem).ok_or_else(|| format!("{}: the port accepts", r.column))?;
                    let shift = r.compat.editor_shift();
                    let at = |p: nomi_base::diagnostic::code::Position| {
                        format!("{}:{}", p.row + shift, p.col + shift)
                    };
                    let (start, end) = (problem.region.start, problem.region.end);
                    let got = format!("{} {}-{}", problem.title, at(start), at(end));
                    if got == r.cell {
                        Ok(())
                    } else {
                        Err(format!("{}: elm {}, the port {got}", r.column, r.cell))
                    }
                });
                (seed.case_id(case.mutation), outcome)
            })
            .collect()
    });
    Suite::open(common::tests_dir("syntax_titles")).finish(outcomes);
}
