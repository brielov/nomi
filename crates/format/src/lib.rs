//! The formatter: elm-format 0.8.8's output for every module it accepts
//! (docs/architecture.md §5).

mod invariants;
mod lower;
mod markdown;
mod render;
mod tree;

use nomi_syntax::{SyntaxError, Tool, parse_module, validate};

pub use invariants::{
    Invariant, UnknownInvariant, broken_invariants, check_invariants, invariant_mismatch,
    upstream_breaks,
};

/// `text` formatted as elm-format does, or `None` where elm-format
/// rejects it.
pub fn format(text: &str) -> Option<String> {
    format_module(text).ok()
}

/// `text` formatted as elm-format does, or, where elm-format rejects it,
/// the first error in the text.
pub fn format_module(text: &str) -> Result<String, SyntaxError> {
    let parse = parse_module(text);
    let first = |errors: &[SyntaxError]| errors.iter().min_by_key(|e| e.range.start()).copied();
    if let Some(error) = first(parse.errors()) {
        return Err(error);
    }
    if let Some(error) = first(&validate(&parse, Tool::ElmFormat)) {
        return Err(error);
    }
    let module = lower::module(&parse.syntax(), text);
    Ok(render::module(&module))
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
