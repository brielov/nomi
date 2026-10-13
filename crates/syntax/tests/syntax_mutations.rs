//! The mutation side of the syntax ratchet: each mutation of each seed in
//! `syntax_mutations/seeds.txt` is a case, passing when nomi accepts and
//! rejects it as every tool in `syntax_mutations/oracle/<seed>.tsv` did.
//! `cargo xtask oracle syntax mutations` stores those tables.

mod common;

use nomi_syntax::Tool;
use nomi_testkit::ratchet::Suite;

#[test]
fn syntax_mutations() {
    let outcomes = common::each_seed(|seed| seed_outcomes(&seed));
    Suite::open(common::dir()).finish(outcomes);
}

type Outcome = (String, Result<(), String>);

fn seed_outcomes(seed: &common::Seed) -> Vec<Outcome> {
    let mut outcomes = Vec::with_capacity(seed.cases.len());
    for (m, cells) in &seed.cases {
        let parse = nomi_syntax::parse_module(&m.source);
        // Releases rarely differ; validate each tool once.
        let mut verdicts: Vec<(Tool, bool)> = Vec::new();
        let mut accepts = |tool: Tool| {
            if let Some(&(_, accepted)) = verdicts.iter().find(|(t, _)| *t == tool) {
                accepted
            } else {
                let accepted =
                    parse.errors().is_empty() && nomi_syntax::validate(&parse, tool).is_empty();
                verdicts.push((tool, accepted));
                accepted
            }
        };
        let mismatch = seed
            .columns
            .iter()
            .zip(cells)
            .find_map(
                |((column, tool), cell)| match (cell.as_str(), accepts(*tool)) {
                    // A crash gives no verdict to match.
                    ("crash", _) => None,
                    ("ok", false) => Some(format!("{column} accepts, nomi rejects")),
                    (_, true) if cell != "ok" => {
                        Some(format!("{column} rejects ({cell}), nomi accepts"))
                    }
                    _ => None,
                },
            );
        outcomes.push((
            format!("{} / {}", seed.name, m.name),
            mismatch.map_or(Ok(()), Err),
        ));
    }
    outcomes
}
