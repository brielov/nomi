//! JSON as upstream reads and writes it, not as the standard has it: the
//! parser takes only what `Json/Decode.hs` takes, reports its errors where
//! it does for each Compat, and leaves strings as written, escapes and
//! all, so that writing them back with [`encode`] reproduces them.

pub mod cursor;
pub mod decode;
pub mod encode;
mod parse;

pub use cursor::{Cursor, Position, Region};
pub use parse::{Kind, ParseError, ParseErrorKind, Snippet, StringProblem, Value, parse};

#[cfg(test)]
mod tests;
