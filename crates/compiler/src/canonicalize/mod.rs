//! Canonicalization (`Canonicalize/*.hs`): what each name a module
//! writes means. It follows upstream's order and its `Result`'s two
//! ways of combining work, so a module shows the errors upstream shows,
//! in its order.

pub mod dups;
pub mod env;
pub mod error;
pub mod foreign;
pub mod types;

#[cfg(test)]
mod tests;
