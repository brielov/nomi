//! The recursion checks over a module's values and a `let`'s
//! definitions (`Module.hs`'s `detectCycles`, `Expression.hs`'s), which
//! plan 5.12 ports; until then every graph passes.

use nomi_base::Compat;

use super::error::R;
use super::expr::LetNode;

/// `detectCycles` over a `let`'s graph.
pub fn let_cycles(_compat: Compat, _nodes: Vec<LetNode>) -> R<()> {
    Ok(())
}
