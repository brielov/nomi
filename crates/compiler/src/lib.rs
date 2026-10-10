//! The semantic core (architecture §6): the HIR modules lower to, and the
//! phases over it, ported from upstream's `compiler/` as each milestone
//! needs them.

pub mod canonical;
pub mod canonicalize;
mod encoding;
pub mod graph;
pub mod hir;
pub mod interface;
pub mod types;
