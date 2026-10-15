//! A port of elm's own parser, `Parse/*.hs` from 0.19.3 with the
//! differences 0.19.1 keeps, run only for its errors: nomi's grammar is
//! the parser, and this says what elm would report of a module that
//! grammar rejects (DECISIONS.md, 2026-10-08).
//!
//! It builds no tree. Each parser returns only what upstream's control
//! flow reads back: where an expression ended, a name a later check
//! compares, whether a number was a float, where the port `checkModule`
//! reports is. 0.19.2 parses as 0.19.3 does.

use nomi_base::Compat;

use crate::Source;

mod declaration;
pub mod docs;
mod engine;
mod error;
mod expression;
mod literal;
mod module;
mod pattern;
mod report;
mod types;

pub use error::Error;
pub(crate) use literal::{Dots, chomp_hex, chomp_number, eat_unicode};
pub use report::{docs_problem, to_problem};

/// What elm `compat` reports first of the module `src`, from a project
/// of kind `source`, if it parses at all.
pub fn parse(src: &[u8], source: Source, compat: Compat) -> Result<(), Error> {
    let mut p = engine::P::new(src, compat);
    module::module(&mut p, source)
}
