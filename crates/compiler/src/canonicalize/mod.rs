//! Canonicalization (`Canonicalize/*.hs`): what each name a module
//! writes means. It follows upstream's order and its `Result`'s two
//! ways of combining work, so a module shows the errors upstream shows,
//! in its order.

mod binops;
mod cycles;
mod dups;
pub(crate) mod effects;
mod env;
pub mod error;
mod expr;
pub mod foreign;
mod local;
pub mod module;
mod pattern;
pub mod references;
pub mod report;
mod shader;
mod types;

#[cfg(test)]
mod tests;
