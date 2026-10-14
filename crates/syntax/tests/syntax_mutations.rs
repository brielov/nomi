//! The mutation side of the syntax ratchet: each mutation of each seed in
//! `syntax_mutations/seeds.txt` is a case, passing when nomi accepts and
//! rejects it as every tool in `syntax_mutations/oracle/<seed>.tsv` did.
//! `cargo xtask oracle syntax mutations` stores those tables.

mod common;

use common::Verdict;
use nomi_testkit::ratchet::Suite;

#[test]
fn syntax_mutations() {
    let outcomes = common::each_seed(|seed| seed_outcomes(&seed));
    Suite::open(common::dir()).finish(outcomes);
}

type Outcome = (String, Result<(), String>);

fn seed_outcomes(seed: &common::Seed) -> Vec<Outcome> {
    let mut outcomes = Vec::with_capacity(seed.cases.len());
    for (m, verdicts) in &seed.cases {
        let parse = nomi_syntax::parse_module(&m.source);
        let accepts =
            |tool| parse.errors().is_empty() && nomi_syntax::validate(&parse, tool).is_empty();
        let mut cells = seed.columns.iter().zip(verdicts);
        let mismatch = cells.find_map(|((column, tool), verdict)| match verdict {
            Verdict::Accepted if !accepts(*tool) => Some(format!("{column} accepts, nomi rejects")),
            Verdict::Rejected(cell) if accepts(*tool) => {
                Some(format!("{column} rejects ({cell}), nomi accepts"))
            }
            Verdict::Accepted | Verdict::Rejected(_) | Verdict::Crashed => None,
        });
        outcomes.push((seed.case_id(m), mismatch.map_or(Ok(()), Err)));
    }
    outcomes
}
