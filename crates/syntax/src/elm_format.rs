//! A port of elm-format's parser, `Parse/*.hs` from 0.8.8 parsing 0.19
//! syntax, run only for its errors: nomi's grammar is the parser, and
//! this says where elm-format would report a module that grammar rejects
//! to be unparsable.
//!
//! It builds no tree. Each parser returns only what upstream's control
//! flow reads back: whether a pattern is a variable, whether a comment
//! sat inside parentheses. Rows and columns are `Word16`s, as upstream's,
//! and wrap as they do.

use nomi_base::ElmPosition;

mod engine;
mod expression;
mod helpers;
mod literal;
mod module;
mod pattern;
mod types;
mod whitespace;

/// Why elm-format would not parse a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The position it reports.
    Position(ElmPosition),
    /// A Haskell `error`: elm-format crashes, with this message.
    Crash(&'static str),
}

/// Where elm-format would report the module `src` unparsable, if it
/// would.
pub fn parse(src: &str) -> Result<(), Error> {
    let mut p = engine::P::new(src);
    module::module(&mut p).map_err(|fail| match fail {
        engine::Fail::At(row, col) => Error::Position(ElmPosition {
            row: u32::from(row),
            column: u32::from(col),
        }),
        engine::Fail::Crash(message) => Error::Crash(message),
    })
}
