//! Types, such as `Maybe (List a) -> { r | name : String }`.

use super::{expect_indented, qualified_name, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, SyntaxErrorKind};

/// Whether a type starts here and continues the construct being parsed.
pub(super) fn at_type(p: &mut Parser<'_>) -> bool {
    matches!(
        p.current(),
        K::UpperIdent | K::LowerIdent | K::LParen | K::LBrace
    ) && p.at_indented()
}

/// A whole type, arrows included. `a -> b -> c` nests to the right.
pub(super) fn type_(p: &mut Parser<'_>) {
    p.guarded(function_type);
}

fn function_type(p: &mut Parser<'_>) {
    let param = applied_type(p);
    if p.at_symbol(K::Arrow) && p.at_indented() {
        let m = param.precede(p);
        p.bump_symbol(K::Arrow);
        type_(p);
        m.complete(p, K::FunctionType);
    }
}

/// A type with its arguments: `Dict String (List a)`.
fn applied_type(p: &mut Parser<'_>) -> CompletedMarker {
    if !at_type(p) {
        p.error(SyntaxErrorKind::ExpectedType);
        return p.missing();
    }
    let (m, kind) = match p.current() {
        K::UpperIdent => {
            let m = p.start();
            type_name(p);
            (m, K::NamedType)
        }
        K::LParen if at_tuple_constructor(p) => {
            let m = p.start();
            p.bump();
            while p.at(K::Comma) {
                p.bump();
            }
            p.bump();
            (m, K::TupleConstructorType)
        }
        _ => return type_atom(p),
    };
    while at_type(p) {
        type_atom(p);
    }
    m.complete(p, kind)
}

/// A type that needs no parentheses as an argument.
pub(super) fn type_atom(p: &mut Parser<'_>) -> CompletedMarker {
    match p.current() {
        K::UpperIdent => {
            let m = p.start();
            type_name(p);
            m.complete(p, K::NamedType)
        }
        K::LowerIdent => {
            let m = p.start();
            p.bump();
            m.complete(p, K::TypeVar)
        }
        K::LParen => paren_type(p),
        K::LBrace => record_type(p),
        _ => unreachable!("at_type checked the token"),
    }
}

/// `Maybe` or `Maybe.Maybe`.
fn type_name(p: &mut Parser<'_>) {
    qualified_name(p, &[K::UpperIdent]);
}

/// `(,)`, `(,,)` and so on.
fn at_tuple_constructor(p: &mut Parser<'_>) -> bool {
    let mut n = 1;
    while p.nth(n) == K::Comma {
        n += 1;
    }
    n > 1 && p.nth(n) == K::RParen
}

/// `()`, `(a)` or `(a, b, c)`. elm wants the commas and the closing
/// parenthesis indented; elm-format doesn't.
fn paren_type(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.eat(K::RParen) {
        return m.complete(p, K::UnitType);
    }
    type_(p);
    let mut kind = K::ParenType;
    loop {
        if p.at(K::RParen) {
            p.note_unindented();
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.note_unindented();
            p.bump();
            kind = K::TupleType;
            type_(p);
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
    m.complete(p, kind)
}

/// `{}`, `{ name : String }` or `{ r | name : String }`.
fn record_type(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.at(K::RBrace) {
        p.check_indented();
        p.bump();
        return m.complete(p, K::RecordType);
    }
    if p.at(K::LowerIdent) && p.nth_at_symbol(1, K::Pipe) {
        let extension = p.start();
        p.check_indented();
        p.bump();
        expect_indented(p, K::Pipe);
        extension.complete(p, K::RecordExtension);
    }
    field_type(p);
    loop {
        if p.at(K::RBrace) {
            p.check_indented();
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            field_type(p);
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
    m.complete(p, K::RecordType)
}

/// `name : String`, or elm-format's legacy `name = String`.
fn field_type(p: &mut Parser<'_>) {
    if !p.at(K::LowerIdent) {
        p.error(SyntaxErrorKind::Expected(K::LowerIdent));
        p.missing();
        return;
    }
    let m = p.start();
    p.check_indented();
    p.bump();
    if p.at_symbol(K::Eq) {
        p.check_indented();
        p.bump_symbol(K::Eq);
    } else {
        expect_indented(p, K::Colon);
    }
    type_(p);
    m.complete(p, K::FieldType);
}

#[cfg(test)]
mod tests;
