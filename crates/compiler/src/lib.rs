//! The semantic core (architecture §6): the HIR modules lower to, and the
//! phases over it, ported from upstream's `compiler/` as each milestone
//! needs them.

pub mod canonical;
mod encoding;
pub mod hir;
pub mod interface;
