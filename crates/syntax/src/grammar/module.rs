//! The module header, the module documentation, imports and the
//! declarations that follow.

use super::declarations::declarations;
use super::{expect_indented, is_parenthesized_op, module_name, parenthesized_op, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{Parser, Scan, SyntaxErrorKind};

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
    let mut scan = p.scan(1);
    loop {
        match (scan.nth(0), scan.nth(1), scan.nth(2)) {
            (K::DotDot | K::LowerIdent, ..) => scan.skip(1),
            (K::UpperIdent, K::LParen, _) => {
                scan.skip(2);
                if !skip_variants(&mut scan) {
                    return false;
                }
            }
            (K::UpperIdent, ..) => scan.skip(1),
            (K::LParen, op, K::RParen) if is_parenthesized_op(op) => scan.skip(3),
            _ => return false,
        }
        match scan.nth(0) {
            K::Comma => scan.skip(1),
            K::RParen => return true,
            _ => return false,
        }
    }
}

/// Whether the variants `scan` is at close, and if so, past their `)`.
fn skip_variants(scan: &mut Scan<'_, '_>) -> bool {
    if scan.nth(0) == K::DotDot {
        if scan.nth(1) != K::RParen {
            return false;
        }
        scan.skip(2);
        return true;
    }
    loop {
        if scan.nth(0) != K::UpperIdent {
            return false;
        }
        match scan.nth(1) {
            K::Comma => scan.skip(2),
            K::RParen => {
                scan.skip(2);
                return true;
            }
            _ => return false,
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
        exposed_items(p, Delimiters::Parenthesized);
        expect_indented(p, K::RParen);
    } else if p.at_indented() && matches!(p.current(), K::DotDot | K::LowerIdent | K::UpperIdent) {
        exposed_items(p, Delimiters::Bare);
    } else {
        p.error(SyntaxErrorKind::Expected(K::LParen));
    }
    m.complete(p, K::ExposingList);
}

/// Whether an exposing list stands in parentheses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delimiters {
    Parenthesized,
    /// An import's list without them, which elm-format takes, as in
    /// `import A exposing a`.
    Bare,
}

/// `..` or a comma-separated list. In parentheses, junk before the next
/// comma is skipped.
fn exposed_items(p: &mut Parser<'_>, delimiters: Delimiters) {
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
        } else if delimiters == Delimiters::Parenthesized
            && p.at_indented_strictly()
            && !p.at(K::RParen)
        {
            p.error_and_bump(SyntaxErrorKind::Expected(K::RParen));
        } else {
            break;
        }
    }
}

fn exposed(p: &mut Parser<'_>) {
    if !p.at_indented() {
        p.missing(SyntaxErrorKind::ExpectedExposed);
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
            if is_parenthesized_op(p.current()) {
                parenthesized_op(p);
            } else {
                p.missing(SyntaxErrorKind::Expected(K::Op));
            }
            expect_indented(p, K::RParen);
            m.complete(p, K::ExposedOperator);
        }
        K::Comma | K::RParen => {
            p.missing(SyntaxErrorKind::ExpectedExposed);
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
    expect_indented(p, K::RParen);
    m.complete(p, K::ExposedVariants);
}
