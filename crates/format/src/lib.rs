//! The formatter: elm-format 0.8.8's output for every module it accepts
//! (docs/architecture.md §5).
//!
//! Rendering arrives in pieces (plan 2.2 to 2.13); until a piece lands,
//! its part of the module keeps the source's text, so a module that is
//! already formatted comes back unchanged from the start.

// Rendering builds on these from plan 2.8; until then only tests do.
#[cfg_attr(not(test), expect(dead_code))]
mod block;
#[cfg_attr(not(test), expect(dead_code))]
mod structure;
#[cfg_attr(not(test), expect(dead_code))]
mod tree;

use nomi_syntax::{Tool, parse_module, validate};

/// `text` formatted as elm-format does, or `None` where elm-format
/// rejects it.
pub fn format(text: &str) -> Option<String> {
    let parse = parse_module(text);
    if !parse.errors().is_empty() || !validate(&parse, Tool::ElmFormat).is_empty() {
        return None;
    }
    Some(text.to_owned())
}

#[cfg(test)]
mod tests;
