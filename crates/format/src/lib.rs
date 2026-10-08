//! The formatter: elm-format 0.8.8's output for every module it accepts
//! (docs/architecture.md §5).

mod block;
mod ghc;
mod lower;
mod markdown;
mod render;
mod structure;
mod tree;

use nomi_syntax::{Tool, parse_module, validate};

/// `text` formatted as elm-format does, or `None` where elm-format
/// rejects it.
pub fn format(text: &str) -> Option<String> {
    let parse = parse_module(text);
    if !parse.errors().is_empty() || !validate(&parse, Tool::ElmFormat).is_empty() {
        return None;
    }
    let module = lower::module(&parse.syntax(), text);
    Some(render::module(&module))
}

#[cfg(test)]
mod tests;
