//! The semantic core (architecture §6): the HIR modules lower to, and the
//! phases over it, ported from upstream's `compiler/`.

pub mod canonical;
pub mod canonicalize;
pub mod docs;
mod encoding;
pub mod graph;
pub mod hir;
pub mod interface;
pub mod nitpick;
pub mod optimize;
pub mod optimized;
pub mod report;
pub mod types;

#[cfg(test)]
mod testing;
