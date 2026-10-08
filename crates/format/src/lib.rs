//! The formatter: elm-format 0.8.8's output for every module it accepts
//! (docs/architecture.md §5).

mod block;
mod ghc;
mod invariants;
mod lower;
mod markdown;
mod render;
mod structure;
mod tree;

pub use invariants::{Invariant, broken_invariants, check_invariants, upstream_breaks};

/// Stack kept free before a recursive step runs, and the size of each new
/// segment when it isn't, as the parser keeps.
const RED_ZONE: usize = 128 * 1024;
const STACK_SEGMENT: usize = 2 * 1024 * 1024;

/// Runs a step of a recursive pass, growing the stack first where little
/// is left, so the depth the parser admits is bounded by memory, not by
/// the thread's stack.
fn guarded<R>(step: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, step)
}
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
