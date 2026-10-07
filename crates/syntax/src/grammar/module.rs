//! The module header, the module documentation, imports and the
//! declarations that follow.

use super::declarations::declarations;
use super::{
    at_parenthesized_op, expect_indented, is_parenthesized_op, module_name, parenthesized_op,
    skip_junk,
};
use crate::SyntaxKind as K;
use crate::parser::{Parser, SyntaxErrorKind};

pub(super) fn module(p: &mut Parser<'_>) {
    let m = p.start();
    if !p.at_fresh_line() {
        p.error(SyntaxErrorKind::ExpectedFreshLine);
    }
    // Each part of the module starts at the margin.
    p.anchor();
    let has_header = at_header(p);
    if has_header {
        header(p);
        skip_junk(p, SyntaxErrorKind::Unexpected, &[]);
    }
    // Without a header, a doc comment documents the first declaration
    // unless imports follow it.
    // skip_junk leaves each of these at a fresh line.
    if p.at(K::DocComment) && (has_header || p.nth(1) == K::ImportKw) {
        p.anchor();
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
    p.anchor();
    p.bump();
    module_name(p);
    if p.at(K::AsKw) {
        let alias = p.start();
        p.bump();
        expect_indented(p, K::UpperIdent);
        alias.complete(p, K::ImportAlias);
    }
    if p.at(K::ExposingKw) || p.at(K::LParen) && p.at_indented_strictly() {
        exposing_list(p);
    } else if p.at(K::LParen) && at_whole_list(p) {
        // elm-format tries a list without `exposing` wherever it stands,
        // even at the margin: `( a, b ) = pair` below an import is one.
        p.anchor();
        exposing_list(p);
    }
    m.complete(p, K::Import);
}

fn at_exposing_list(p: &mut Parser<'_>) -> bool {
    p.at(K::ExposingKw) || (p.at(K::LParen) && p.at_indented())
}

/// Whether a whole parenthesized exposing list is ahead, which elm-format
/// reads wherever it stands.
fn at_whole_list(p: &mut Parser<'_>) -> bool {
    let mut n = 1;
    loop {
        n = match (p.nth(n), p.nth(n + 1), p.nth(n + 2)) {
            (K::DotDot | K::LowerIdent, ..) => n + 1,
            (K::UpperIdent, K::LParen, _) => match variants_end(p, n + 2) {
                Some(end) => end,
                None => return false,
            },
            (K::UpperIdent, ..) => n + 1,
            (K::LParen, op, K::RParen) if is_parenthesized_op(op) => n + 3,
            _ => return false,
        };
        match p.nth(n) {
            K::Comma => n += 1,
            K::RParen => return true,
            _ => return false,
        }
    }
}

/// Past the `)` of the variants starting at token `n`, if they close.
fn variants_end(p: &mut Parser<'_>, mut n: usize) -> Option<usize> {
    if p.nth(n) == K::DotDot {
        return (p.nth(n + 1) == K::RParen).then_some(n + 2);
    }
    loop {
        if p.nth(n) != K::UpperIdent {
            return None;
        }
        match p.nth(n + 1) {
            K::Comma => n += 2,
            K::RParen => return Some(n + 2),
            _ => return None,
        }
    }
}

/// `exposing (a, B(..), (+))` or `exposing (..)`. elm-format also takes a
/// list without `exposing`, and an import's list without parentheses.
fn exposing_list(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(K::ExposingKw) {
        p.bump();
    }
    if p.at(K::LParen) {
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
    // Both tools read `..` by its characters: `(..--x` is `(..` and a
    // comment.
    if p.at_symbol(K::DotDot) {
        p.bump_symbol(K::DotDot);
        return;
    }
    exposed(p);
    loop {
        if p.at(K::Comma) && p.at_indented() {
            p.bump();
            exposed(p);
        } else if parenthesized && p.at_indented_strictly() && !p.at(K::RParen) {
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
    // elm-format reads `..` by its characters, elm an operator, so only
    // elm-format takes a line comment right after it.
    if p.at_symbol(K::DotDot) {
        p.bump_symbol(K::DotDot);
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
