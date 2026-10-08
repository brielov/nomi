//! What the port of elm's syntax reports makes of each mutation some elm
//! rejects, for the two ratchets on them: one on titles and regions, one
//! on whole reports.

use nomi_base::Compat;
use nomi_base::diagnostic::{Problem, code};
use nomi_syntax::{Tool, elm};

use crate::common::Seed;

/// One elm's rejection of a mutation.
pub struct Rejection {
    /// The column, as `elm 0.19.1`.
    pub column: String,
    pub compat: Compat,
    /// The title and region elm reported, as the oracle table keeps them.
    pub cell: String,
    /// The port's report, or why there is none.
    pub problem: Result<Problem, String>,
}

/// Each mutation of `seed` some elm rejects, by case id, with each
/// rejection. Crashes give no report to match.
pub fn rejections(seed: &Seed) -> Vec<(String, Vec<Rejection>)> {
    let mut cases = Vec::new();
    for (m, cells) in &seed.cases {
        let source = code::Source::new(&m.source);
        let rejections: Vec<Rejection> = (seed.columns.iter().zip(cells))
            .filter_map(|((column, tool), cell)| {
                let Tool::Elm(kind, compat) = *tool else {
                    return None;
                };
                if cell == "ok" || cell == "crash" {
                    return None;
                }
                let problem = match elm::parse(m.source.as_bytes(), kind, compat) {
                    Ok(()) => Err("the port accepts".to_owned()),
                    Err(error) => elm::to_problem(&source, &error, compat)
                        .ok_or_else(|| format!("no report yet for {error:?}")),
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
            cases.push((format!("{} / {}", seed.name, m.name), rejections));
        }
    }
    cases
}
