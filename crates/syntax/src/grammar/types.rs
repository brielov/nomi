//! Types, such as `Maybe (List a) -> { r | name : String }`.

use super::{
    at_tuple_constructor, expect_indented, qualified_name, sequence_tail, tuple_constructor_tail,
};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, SyntaxErrorKind};

/// Whether a type starts here and continues the construct being parsed.
pub(super) fn at_type(p: &mut Parser<'_>) -> bool {
    starts_type(p.current()) && p.at_indented()
}

/// Whether another argument starts here, which must be indented in either
/// layout.
pub(super) fn at_type_argument(p: &mut Parser<'_>) -> bool {
    starts_type(p.current()) && p.at_indented_strictly()
}

fn starts_type(kind: K) -> bool {
    matches!(kind, K::UpperIdent | K::LowerIdent | K::LParen | K::LBrace)
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
        return p.missing(SyntaxErrorKind::ExpectedType);
    }
    let (m, kind) = match p.current() {
        K::UpperIdent => {
            let m = p.start();
            qualified_name(p, &[K::UpperIdent]);
            (m, K::NamedType)
        }
        K::LParen if at_tuple_constructor(p, 1) => {
            let m = p.start();
            p.bump();
            tuple_constructor_tail(p);
            (m, K::TupleConstructorType)
        }
        _ => return type_atom(p),
    };
    while at_type_argument(p) {
        type_atom(p);
    }
    m.complete(p, kind)
}

/// A type that needs no parentheses as an argument.
pub(super) fn type_atom(p: &mut Parser<'_>) -> CompletedMarker {
    match p.current() {
        K::UpperIdent => {
            let m = p.start();
            qualified_name(p, &[K::UpperIdent]);
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

/// `()`, `(a)` or `(a, b, c)`.
fn paren_type(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.eat(K::RParen) {
        return m.complete(p, K::UnitType);
    }
    type_(p);
    let kind = if sequence_tail(p, K::RParen, type_) {
        K::TupleType
    } else {
        K::ParenType
    };
    m.complete(p, kind)
}

/// `{}`, `{ name : String }` or `{ r | name : String }`.
fn record_type(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if p.at(K::RBrace) {
        p.bump();
        return m.complete(p, K::RecordType);
    }
    if p.at(K::LowerIdent) && p.nth_at_symbol(1, K::Pipe) {
        let extension = p.start();
        p.bump();
        expect_indented(p, K::Pipe);
        extension.complete(p, K::RecordExtension);
    }
    field_type(p);
    sequence_tail(p, K::RBrace, field_type);
    m.complete(p, K::RecordType)
}

/// `name : String`, or elm-format's legacy `name = String`.
fn field_type(p: &mut Parser<'_>) {
    if !p.at(K::LowerIdent) {
        p.missing(SyntaxErrorKind::Expected(K::LowerIdent));
        return;
    }
    let m = p.start();
    p.bump();
    if p.at_symbol(K::Eq) {
        p.bump_symbol(K::Eq);
    } else {
        expect_indented(p, K::Colon);
    }
    type_(p);
    m.complete(p, K::FieldType);
}

#[cfg(test)]
mod tests;
