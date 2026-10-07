//! Patterns, such as `(Just x) :: rest as all`.

use super::{expect_indented, qualified_name, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, SyntaxErrorKind};

/// Whether a pattern that needs no parentheses as an argument starts here
/// and continues the construct being parsed.
pub(super) fn at_pattern_atom(p: &mut Parser<'_>) -> bool {
    let starts = match p.current() {
        K::LowerIdent
        | K::UpperIdent
        | K::Underscore
        | K::Int
        | K::Float
        | K::Char
        | K::String
        | K::MultilineString
        | K::LParen
        | K::LBracket
        | K::LBrace => true,
        K::Minus => at_negative_literal(p),
        _ => false,
    };
    starts && p.at_indented()
}

fn at_negative_literal(p: &mut Parser<'_>) -> bool {
    matches!(p.nth(1), K::Int | K::Float) && p.nth_is_joined(1)
}

/// A whole pattern: constructors with arguments, `::` and `as`. `a :: b
/// :: c` nests to the right, and `as` takes everything before it.
pub(super) fn pattern(p: &mut Parser<'_>) {
    let Some(cons) = p.guarded(cons_pattern) else {
        return;
    };
    if p.at(K::AsKw) && p.at_indented() {
        let m = cons.precede(p);
        p.bump();
        expect_indented(p, K::LowerIdent);
        m.complete(p, K::AliasPattern);
    }
}

fn cons_pattern(p: &mut Parser<'_>) -> CompletedMarker {
    let head = applied_pattern(p);
    if p.at_symbol(K::ColonColon) && p.at_indented() {
        let m = head.precede(p);
        p.bump_symbol(K::ColonColon);
        p.guarded(cons_pattern);
        return m.complete(p, K::ConsPattern);
    }
    head
}

/// A constructor with its arguments, or an atom.
fn applied_pattern(p: &mut Parser<'_>) -> CompletedMarker {
    if !at_pattern_atom(p) {
        p.error(SyntaxErrorKind::ExpectedPattern);
        return p.missing();
    }
    if !p.at(K::UpperIdent) {
        return pattern_atom(p);
    }
    let m = p.start();
    qualified_name(p, &[K::UpperIdent]);
    while at_pattern_atom(p) {
        pattern_atom(p);
    }
    m.complete(p, K::ConstructorPattern)
}

/// A pattern that needs no parentheses as an argument, such as a
/// function's parameter. The caller checks [`at_pattern_atom`].
pub(super) fn pattern_atom(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let kind = match p.current() {
        K::LowerIdent => {
            p.bump();
            K::VarPattern
        }
        K::UpperIdent => {
            qualified_name(p, &[K::UpperIdent]);
            K::ConstructorPattern
        }
        K::Underscore => {
            p.bump();
            if matches!(p.current(), K::LowerIdent | K::UpperIdent | K::Int) && p.nth_is_joined(0) {
                p.error_and_bump(SyntaxErrorKind::NamedWildcard);
            }
            K::WildcardPattern
        }
        K::Minus => {
            p.bump();
            p.bump();
            K::LiteralPattern
        }
        K::Int | K::Float | K::Char | K::String | K::MultilineString => {
            p.bump();
            K::LiteralPattern
        }
        K::LParen => paren_pattern(p),
        K::LBracket => list_pattern(p),
        K::LBrace => record_pattern(p),
        _ => unreachable!("at_pattern_atom checked the token"),
    };
    m.complete(p, kind)
}

/// `()`, `(p)` or `(a, b)`. Returns the kind for the caller's node.
// TODO(plan 1.15): elm wants the closing parenthesis indented;
// elm-format doesn't.
fn paren_pattern(p: &mut Parser<'_>) -> K {
    p.bump();
    if p.at(K::RParen) {
        p.check_indented();
        p.bump();
        return K::UnitPattern;
    }
    pattern(p);
    let mut kind = K::ParenPattern;
    loop {
        if p.eat(K::RParen) {
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            kind = K::TuplePattern;
            pattern(p);
        } else if p.at_indented() {
            skip_junk(
                p,
                SyntaxErrorKind::Expected(K::RParen),
                &[K::Comma, K::RParen],
            );
        } else {
            p.error(SyntaxErrorKind::Expected(K::RParen));
            break;
        }
    }
    kind
}

/// `[]` or `[a, b]`.
fn list_pattern(p: &mut Parser<'_>) -> K {
    p.bump();
    if !p.at(K::RBracket) {
        pattern(p);
    }
    loop {
        if p.at(K::RBracket) {
            p.check_indented();
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            pattern(p);
        } else if p.at_indented() {
            skip_junk(
                p,
                SyntaxErrorKind::Expected(K::RBracket),
                &[K::Comma, K::RBracket],
            );
        } else {
            p.error(SyntaxErrorKind::Expected(K::RBracket));
            break;
        }
    }
    K::ListPattern
}

/// `{}` or `{ a, b }`.
fn record_pattern(p: &mut Parser<'_>) -> K {
    p.bump();
    if !p.at(K::RBrace) {
        field_pattern(p);
    }
    loop {
        if p.at(K::RBrace) {
            p.check_indented();
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            field_pattern(p);
        } else if p.at_indented() {
            skip_junk(
                p,
                SyntaxErrorKind::Expected(K::RBrace),
                &[K::Comma, K::RBrace],
            );
        } else {
            p.error(SyntaxErrorKind::Expected(K::RBrace));
            break;
        }
    }
    K::RecordPattern
}

fn field_pattern(p: &mut Parser<'_>) {
    if !p.at(K::LowerIdent) {
        p.error(SyntaxErrorKind::Expected(K::LowerIdent));
        p.missing();
        return;
    }
    let m = p.start();
    p.check_indented();
    p.bump();
    m.complete(p, K::FieldPattern);
}

#[cfg(test)]
mod tests;
