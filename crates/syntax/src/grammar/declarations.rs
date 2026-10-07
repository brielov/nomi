//! Top-level declarations: values, custom types, type aliases, ports and
//! infix declarations, each with the doc comment above it.

use super::definitions::value;
use super::expressions::expr;
use super::patterns::{at_pattern_atom, pattern_atom};
use super::types::{at_type, type_, type_atom};
use super::{expect_indented, expect_part};
use crate::SyntaxKind as K;
use crate::parser::{Marker, Parser, SyntaxErrorKind};

/// Whether a declaration could start here: a fresh line, then a token
/// that begins one.
fn at_declaration(p: &mut Parser<'_>) -> bool {
    p.at_fresh_line()
        && (matches!(
            p.current(),
            K::DocComment | K::TypeKw | K::PortKw | K::LowerIdent
        ) || at_pattern_atom(p))
}

/// One declaration, which starts on a fresh line. Anything else is
/// skipped, with one error, up to where a declaration could start.
pub(super) fn declaration(p: &mut Parser<'_>) {
    if !p.at_fresh_line() {
        p.error(SyntaxErrorKind::ExpectedFreshLine);
    }
    let m = p.start();
    if p.at(K::DocComment) {
        p.bump();
        if p.at(K::Eof) {
            p.error(SyntaxErrorKind::ExpectedDeclaration);
            m.complete(p, K::Error);
            return;
        }
        if !p.at_fresh_line() {
            p.error(SyntaxErrorKind::ExpectedFreshLine);
        }
    }
    // The first token sits at the margin; the rest must be right of it.
    p.anchor();
    match p.current() {
        K::TypeKw if p.nth_at_contextual(1, K::AliasKw) => type_alias(p, m),
        K::TypeKw => custom_type(p, m),
        K::PortKw => port(p, m),
        K::LowerIdent if at_infix(p) => infix(p, m),
        K::LowerIdent => {
            value(p);
            m.complete(p, K::ValueDecl);
        }
        // TODO(plan 1.15): elm-format takes `( a, b ) = pair` here; elm
        // wants a name.
        _ if at_pattern_atom(p) => {
            pattern_atom(p);
            if expect_part(p, K::Eq) {
                expr(p);
            }
            m.complete(p, K::DestructureDecl);
        }
        _ => {
            p.error(SyntaxErrorKind::ExpectedDeclaration);
            p.bump();
            while !p.at(K::Eof) && !at_declaration(p) {
                p.bump();
            }
            m.complete(p, K::Error);
        }
    }
}

/// `type alias Name a = Type`.
fn type_alias(p: &mut Parser<'_>, m: Marker) {
    p.bump();
    p.check_indented();
    p.bump_as(K::AliasKw);
    type_head(p);
    if expect_part(p, K::Eq) {
        type_(p);
    }
    m.complete(p, K::TypeAliasDecl);
}

/// `type Name a = A | B a`.
fn custom_type(p: &mut Parser<'_>, m: Marker) {
    p.bump();
    type_head(p);
    if expect_part(p, K::Eq) {
        variant(p);
        while p.at_symbol(K::Pipe) && p.at_indented() {
            p.bump_symbol(K::Pipe);
            variant(p);
        }
    }
    m.complete(p, K::CustomTypeDecl);
}

/// The name of a type being declared and its parameters.
fn type_head(p: &mut Parser<'_>) {
    expect_indented(p, K::UpperIdent);
    while p.at(K::LowerIdent) && p.at_indented() {
        let param = p.start();
        p.bump();
        param.complete(p, K::TypeParam);
    }
}

/// `Just a`: a constructor and the types it holds, each an atom.
fn variant(p: &mut Parser<'_>) {
    if !(p.at(K::UpperIdent) && p.at_indented()) {
        p.error(SyntaxErrorKind::Expected(K::UpperIdent));
        p.missing();
        return;
    }
    let m = p.start();
    p.bump();
    while at_type(p) {
        type_atom(p);
    }
    m.complete(p, K::Variant);
}

/// `port send : String -> Cmd msg`.
fn port(p: &mut Parser<'_>, m: Marker) {
    p.bump();
    expect_indented(p, K::LowerIdent);
    if expect_part(p, K::Colon) {
        type_(p);
    }
    m.complete(p, K::PortDecl);
}

/// Upstream reads infix declarations only in kernel packages, before the
/// other declarations; anywhere else `infix` is a name. Validation flags
/// them out of place, so the grammar takes them where no value could be:
/// `infix left 5`, an associativity then a precedence.
fn at_infix(p: &mut Parser<'_>) -> bool {
    p.nth_at_contextual(0, K::InfixKw)
        && [K::LeftKw, K::RightKw, K::NonKw]
            .into_iter()
            .any(|kind| p.nth_at_contextual(1, kind))
        && p.nth(2) == K::Int
}

/// `infix left 5 (+) = add`.
fn infix(p: &mut Parser<'_>, m: Marker) {
    p.bump_as(K::InfixKw);
    let associativity = [K::LeftKw, K::RightKw, K::NonKw]
        .into_iter()
        .find(|&kind| p.nth_at_contextual(0, kind))
        .expect("at_infix checked the associativity");
    p.check_indented();
    p.bump_as(associativity);
    expect_indented(p, K::Int);
    if expect_indented(p, K::LParen) {
        if matches!(p.current(), K::Operator | K::Minus | K::ColonColon) && p.at_indented() {
            let op = p.start();
            p.bump();
            op.complete(p, K::Op);
        } else {
            p.error(SyntaxErrorKind::Expected(K::Op));
            p.missing();
        }
        expect_indented(p, K::RParen);
    }
    if expect_part(p, K::Eq) {
        expect_indented(p, K::LowerIdent);
    }
    m.complete(p, K::InfixDecl);
}
