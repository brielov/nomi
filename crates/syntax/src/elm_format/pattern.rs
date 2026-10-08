//! `Parse/Pattern.hs`. A pattern's value is whether it is a variable,
//! all that a definition reads of one.

use super::engine::{P, R, deep};
use super::helpers::{
    cap_var, cons, dot_sep1, low_var, reserved, separated, space_prefix, surround__,
};
use super::literal::literal;
use super::whitespace::whitespace;

fn basic(p: &mut P<'_>) -> R<bool> {
    p.choice(&mut [
        &mut |p| p.char('_').map(|()| false),
        &mut |p| low_var(p).map(|_| true),
        &mut |p| dot_sep1(p, cap_var).map(|_| false),
        &mut |p| literal(p).map(|()| false),
    ])
}

/// `asPattern`: `parser`, perhaps `as` a name.
fn as_pattern<'s>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> R<bool>) -> R<bool> {
    let is_var = parser(p)?;
    let alias = p.option(|p| {
        p.attempt(|p| {
            whitespace(p)?;
            reserved(p, "as")
        })?;
        whitespace(p)?;
        low_var(p)
    })?;
    Ok(is_var && alias.is_none())
}

fn record(p: &mut P<'_>) -> R<bool> {
    surround__(p, '{', '}', &mut |p| low_var(p).map(drop)).map(|_| false)
}

/// `tuple`: in parentheses with no comments, one pattern is itself.
fn tuple(p: &mut P<'_>) -> R<bool> {
    let patterns = surround__(p, '(', ')', &mut expr)?;
    Ok(matches!(patterns[..], [(false, true)]))
}

fn list(p: &mut P<'_>) -> R<bool> {
    surround__(p, '[', ']', &mut |p| expr(p).map(drop)).map(|_| false)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> R<bool> {
    deep(p, |p| {
        p.choice(&mut [&mut record, &mut tuple, &mut list, &mut basic])
    })
}

fn pattern_constructor(p: &mut P<'_>) -> R<bool> {
    let (n, last) = dot_sep1(p, cap_var)?;
    if !(n == 1 && matches!(last, "True" | "False")) {
        space_prefix(p, &mut term)?;
    }
    Ok(false)
}

/// `expr`.
pub(super) fn expr(p: &mut P<'_>) -> R<bool> {
    deep(p, |p| {
        as_pattern(p, |p| {
            let one = separated(p, cons, &mut |p| p.alt(pattern_constructor, term))?;
            Ok(one == Some(true))
        })
    })
}
