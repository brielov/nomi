//! Canonicalization (`Canonicalize/*.hs`): what each name a module
//! writes means. It follows upstream's order and its `Result`'s two
//! ways of combining work, so a module shows the errors upstream shows,
//! in its order.

use std::cmp::Ordering;

use nomi_base::Compat;

pub mod binops;
pub mod cycles;
pub mod dups;
pub mod env;
pub mod error;
pub mod expr;
pub mod foreign;
pub mod local;
pub mod module;
pub mod pattern;
pub mod types;

/// How a release orders names, wherever an order shows: as bytes, and
/// from 0.19.3 shorter names first (`compareFast`).
pub fn name_order(compat: Compat) -> fn(&str, &str) -> Ordering {
    match compat {
        Compat::V0_19_1 | Compat::V0_19_2 => |a, b| a.cmp(b),
        Compat::V0_19_3 => |a, b| (a.len(), a).cmp(&(b.len(), b)),
    }
}

/// `names`, in `compat`'s order.
pub fn sorted<'n>(compat: Compat, names: impl IntoIterator<Item = &'n String>) -> Vec<&'n String> {
    let mut names: Vec<&String> = names.into_iter().collect();
    let order = name_order(compat);
    names.sort_by(|a, b| order(a, b));
    names
}

#[cfg(test)]
mod tests;
