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
    pub rejections: Vec<Rejection<'s>>,
}

/// One elm's rejection of a mutation.
pub(crate) struct Rejection<'s> {
    /// The column, as `elm 0.19.1`.
    pub column: &'s str,
    pub compat: Compat,
    /// The title and region elm reported, as the oracle table keeps them.
    pub cell: &'s str,
    /// The port's report, `None` where the port accepts.
    pub problem: Option<Problem>,
}

/// Each mutation of `seed` some elm rejects. Crashes give no report to
/// match.
pub(crate) fn rejections(seed: &Seed) -> Vec<Case<'_>> {
    let mut cases = Vec::new();
    for (m, verdicts) in &seed.cases {
        let source = code::Source::new(&m.source);
        let rejections: Vec<Rejection<'_>> = (seed.columns.iter().zip(verdicts))
            .filter_map(|((column, tool), verdict)| {
                let (Tool::Elm(kind, compat), Verdict::Rejected(cell)) = (*tool, verdict) else {
                    return None;
                };
                let problem = (elm::parse(m.source.as_bytes(), kind, compat).err())
                    .map(|error| elm::to_problem(&source, &error, compat));
                Some(Rejection {
                    column,
                    compat,
                    cell,
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
