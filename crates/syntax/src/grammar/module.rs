//! The module header, the module documentation, imports and the
//! declarations that follow.

use super::declarations::declarations;
use super::{at_parenthesized_op, expect_indented, module_name, parenthesized_op, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{Parser, SyntaxErrorKind};

pub(super) fn module(p: &mut Parser<'_>) {
    let m = p.start();
    if !p.at_fresh_line() {
        p.error(SyntaxErrorKind::ExpectedFreshLine);
    }
    let has_header = at_header(p);
    if has_header {
        header(p);
        skip_junk(p, SyntaxErrorKind::Unexpected, &[]);
    }
    // Without a header, a doc comment documents the first declaration
    // unless imports follow it.
    // skip_junk leaves each of these at a fresh line.
    if p.at(K::DocComment) && (has_header || p.nth(1) == K::ImportKw) {
        p.bump();
        skip_junk(p, SyntaxErrorKind::Unexpected, &[]);
    }
    while p.at(K::ImportKw) {
        import(p);
        skip_junk(p, SyntaxErrorKind::Unexpected, &[]);
    }
    declarations(p);
    m.complete(p, K::Module);
}

fn at_header(p: &mut Parser<'_>) -> bool {
    p.at(K::ModuleKw)
        || ((p.at(K::PortKw) || p.nth_at_contextual(0, K::EffectKw)) && p.nth(1) == K::ModuleKw)
}

/// `port module`, `effect module … where { … }` or `module`, then the
/// name and the exposing list. elm-format also takes the legacy forms: no
/// exposing list, a list without `exposing`, and a stray `where` after the
/// name or the list.
fn header(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(K::ModuleKw) {
        p.bump();
    } else {
        if p.at(K::PortKw) {
            p.bump();
        } else {
            p.bump_as(K::EffectKw);
        }
        expect_indented(p, K::ModuleKw);
    }
    module_name(p);
    if p.at(K::WhereKw) && p.at_indented() {
        if p.nth(1) == K::LBrace {
            effect_fields(p);
        } else {
            p.bump();
        }
    }
    if at_exposing_list(p) {
        exposing_list(p);
    }
    if p.at(K::WhereKw) && p.at_indented() {
        p.bump();
    }
    m.complete(p, K::ModuleHeader);
}

/// `where { command = MyCmd, subscription = MySub }`. Which fields an
/// effect module needs is the validation pass's concern.
fn effect_fields(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    expect_indented(p, K::LBrace);
    effect_field(p);
    while p.at(K::Comma) && p.at_indented() {
        p.bump();
        effect_field(p);
    }
    expect_indented(p, K::RBrace);
    m.complete(p, K::EffectFields);
}

fn effect_field(p: &mut Parser<'_>) {
    let m = p.start();
    expect_indented(p, K::LowerIdent);
    expect_indented(p, K::Eq);
    expect_indented(p, K::UpperIdent);
    m.complete(p, K::EffectField);
}

/// `import Name as Alias exposing (…)`.
fn import(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    module_name(p);
    if p.at(K::AsKw) {
        let alias = p.start();
        p.note_unindented();
        p.bump();
        expect_indented(p, K::UpperIdent);
        alias.complete(p, K::ImportAlias);
    }
    if at_exposing_list(p) {
        exposing_list(p);
    }
    m.complete(p, K::Import);
}

fn at_exposing_list(p: &mut Parser<'_>) -> bool {
    p.at(K::ExposingKw) || (p.at(K::LParen) && p.at_indented())
}

/// `exposing (a, B(..), (+))` or `exposing (..)`. elm-format also takes a
/// list without `exposing`, and an import's list without parentheses.
fn exposing_list(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(K::ExposingKw) {
        p.note_unindented();
        p.bump();
    }
    if p.at(K::LParen) {
        p.note_unindented();
        p.bump();
        exposed_items(p, true);
        closing_paren(p);
    } else if p.at_indented() && matches!(p.current(), K::DotDot | K::LowerIdent | K::UpperIdent) {
        exposed_items(p, false);
    } else {
        p.error(SyntaxErrorKind::Expected(K::LParen));
    }
    m.complete(p, K::ExposingList);
}

/// `..` or a comma-separated list. In parentheses, junk before the next
/// comma is skipped.
fn exposed_items(p: &mut Parser<'_>, parenthesized: bool) {
    if p.at(K::DotDot) {
        p.note_unindented();
        p.bump();
        return;
    }
    exposed(p);
    loop {
        if p.at(K::Comma) && p.at_indented() {
            p.bump();
            exposed(p);
        } else if parenthesized && p.at_indented() && !p.at(K::RParen) {
            p.error_and_bump(SyntaxErrorKind::Expected(K::RParen));
        } else {
            break;
        }
    }
}

fn exposed(p: &mut Parser<'_>) {
    if !p.at_indented() {
        p.error(SyntaxErrorKind::ExpectedExposed);
        p.missing();
        return;
    }
    match p.current() {
        K::LowerIdent => {
            let m = p.start();
            p.bump();
            m.complete(p, K::ExposedValue);
        }
        K::UpperIdent => {
            let m = p.start();
            p.bump();
            if p.at(K::LParen) && p.at_indented() {
                exposed_variants(p);
            }
            m.complete(p, K::ExposedType);
        }
        K::LParen => {
            let m = p.start();
            p.bump();
            if at_parenthesized_op(p) {
                p.note_unindented();
                parenthesized_op(p);
            } else {
                p.error(SyntaxErrorKind::Expected(K::Op));
                p.missing();
            }
            closing_paren(p);
            m.complete(p, K::ExposedOperator);
        }
        K::Comma | K::RParen => {
            p.error(SyntaxErrorKind::ExpectedExposed);
            p.missing();
        }
        _ => p.error_and_bump(SyntaxErrorKind::ExpectedExposed),
    }
}

/// `(..)`, or elm-format's legacy `(A, B)`.
fn exposed_variants(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    if p.at(K::DotDot) {
        p.note_unindented();
        p.bump();
    } else {
        expect_indented(p, K::UpperIdent);
        while p.at(K::Comma) && p.at_indented() {
            p.bump();
            expect_indented(p, K::UpperIdent);
        }
    }
    closing_paren(p);
    m.complete(p, K::ExposedVariants);
}

fn closing_paren(p: &mut Parser<'_>) {
    expect_indented(p, K::RParen);
}
