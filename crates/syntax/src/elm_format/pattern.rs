//! `Parse/Pattern.hs`. A pattern's value is whether it is a variable,
//! all that a definition reads of one.

use nomi_base::grow_large;

use super::engine::{P, R};
use super::helpers::{
    Commented, Minus, cap_var, cons, dot_sep1, low_var, reserved, separated, space_prefix,
    surround_commented,
};
use super::literal::literal;
use super::whitespace::whitespace;

/// What a definition reads of a pattern: whether it is a variable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Var,
    Other,
}

fn basic(p: &mut P<'_>) -> R<Shape> {
    p.choice(&mut [
        &mut |p| p.char('_').map(|()| Shape::Other),
        &mut |p| low_var(p).map(|_| Shape::Var),
        &mut |p| dot_sep1(p, cap_var).map(|_| Shape::Other),
        &mut |p| literal(p).map(|()| Shape::Other),
    ])
}

/// `asPattern`: `parser`, perhaps `as` a name.
fn as_pattern<'s>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> R<Shape>) -> R<Shape> {
    let shape = parser(p)?;
    let alias = p.option(|p| {
        p.attempt(|p| {
            whitespace(p)?;
            reserved(p, "as")
        })?;
        whitespace(p)?;
        low_var(p)
    })?;
    Ok(match alias {
        None => shape,
        Some(_) => Shape::Other,
    })
}

fn record(p: &mut P<'_>) -> R<Shape> {
    surround_commented(p, '{', '}', &mut |p| low_var(p).map(drop)).map(|_| Shape::Other)
}

/// `tuple`: in parentheses with no comments, one pattern is itself.
fn tuple(p: &mut P<'_>) -> R<Shape> {
    Ok(match surround_commented(p, '(', ')', &mut expr)? {
        Some(Commented {
            commented: false,
            value: shape,
        }) => shape,
        _ => Shape::Other,
    })
}

fn list(p: &mut P<'_>) -> R<Shape> {
    surround_commented(p, '[', ']', &mut |p| expr(p).map(drop)).map(|_| Shape::Other)
}

/// `term`.
pub(super) fn term(p: &mut P<'_>) -> R<Shape> {
    grow_large(|| p.choice(&mut [&mut record, &mut tuple, &mut list, &mut basic]))
}

fn pattern_constructor(p: &mut P<'_>) -> R<Shape> {
    let (n, last) = dot_sep1(p, cap_var)?;
    if !(n == 1 && matches!(last, "True" | "False")) {
        space_prefix(p, Minus::Anywhere, &mut term)?;
    }
    Ok(Shape::Other)
}

/// `expr`.
pub(super) fn expr(p: &mut P<'_>) -> R<Shape> {
    grow_large(|| {
        as_pattern(p, |p| {
            let one = separated(p, cons, &mut |p| p.alt(pattern_constructor, term))?;
            Ok(one.unwrap_or(Shape::Other))
        })
    })
}
