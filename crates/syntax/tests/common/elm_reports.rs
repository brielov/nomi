//! What the port of elm's syntax reports makes of each mutation some elm
//! rejects, for the two ratchets on them: one on titles and regions, one
//! on whole reports.

use nomi_base::Compat;
use nomi_base::diagnostic::{Problem, code};
use nomi_syntax::{Tool, elm};
use nomi_testkit::mutation::Mutation;

use crate::common::{Seed, Verdict};

/// A mutation some elm rejects, with each rejection.
pub(crate) struct Case<'s> {
    pub mutation: &'s Mutation,
    pub rejections: Vec<Rejection>,
}

/// One elm's rejection of a mutation.
pub(crate) struct Rejection {
    /// The column, as `elm 0.19.1`.
    pub column: String,
    pub compat: Compat,
    /// The title and region elm reported, as the oracle table keeps them.
    pub cell: String,
    /// The port's report, or why there is none.
    pub problem: Result<Problem, String>,
}

/// Each mutation of `seed` some elm rejects. Crashes give no report to
/// match.
pub(crate) fn rejections(seed: &Seed) -> Vec<Case<'_>> {
    let mut cases = Vec::new();
    for (m, verdicts) in &seed.cases {
        let source = code::Source::new(&m.source);
        let rejections: Vec<Rejection> = (seed.columns.iter().zip(verdicts))
            .filter_map(|((column, tool), verdict)| {
                let (Tool::Elm(kind, compat), Verdict::Rejected(cell)) = (*tool, verdict) else {
                    return None;
                };
                let problem = match elm::parse(m.source.as_bytes(), kind, compat) {
                    Ok(()) => Err("the port accepts".to_owned()),
                    Err(error) => Ok(elm::to_problem(&source, &error, compat)),
                };
                Some(Rejection {
                    column: column.clone(),
                    compat,
                    cell: cell.clone(),
                    problem,
                })
            })
            .collect();
        if !rejections.is_empty() {
            cases.push(Case {
                mutation: m,
                rejections,
            });
        }
    }
    cases
}
