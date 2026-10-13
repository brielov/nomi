//! Canonicalization (`Canonicalize/*.hs`): what each name a module
//! writes means. It follows upstream's order and its `Result`'s two
//! ways of combining work, so a module shows the errors upstream shows,
//! in its order.

pub mod binops;
pub mod cycles;
pub mod dups;
pub mod effects;
pub mod env;
pub mod error;
pub mod expr;
pub mod foreign;
pub mod local;
pub mod module;
pub mod pattern;
pub mod references;
pub mod report;
pub mod shader;
pub mod types;

#[cfg(test)]
mod tests;
