//! Value definitions with an optional signature, shared by let blocks and
//! the top level.

use super::expressions::expr;
use super::patterns::{at_pattern_argument, pattern_atom};
use super::types::type_;
use super::{expect_indented, expect_part, is_parenthesized_op, parenthesized_op};
use crate::SyntaxKind as K;
use crate::parser::Parser;

/// Whether a value's name is current: a lower name, or elm-format's
/// legacy operator name, as in `(=>) a b = …`.
pub(super) fn at_value_name(p: &mut Parser<'_>) -> bool {
    p.at(K::LowerIdent) || at_operator_name(p)
}

/// elm-format reads the operators an expression may name, `..` and
/// backticked functions aside.
fn at_operator_name(p: &mut Parser<'_>) -> bool {
    let op = p.nth(1);
    p.at(K::LParen)
        && is_parenthesized_op(op)
        && op != K::DotDot
        && p.nth_text(1).is_some_and(|text| !text.starts_with('`'))
        && p.nth(2) == K::RParen
}

fn value_name(p: &mut Parser<'_>) {
    if p.at(K::LowerIdent) {
        p.bump();
        return;
    }
    let m = p.start();
    p.bump();
    parenthesized_op(p);
    p.bump();
    m.complete(p, K::OperatorName);
}

/// `name : Type` on one line and `name args = body` aligned below it, or
/// just one of the two: elm-format takes a signature alone, which elm's
/// validation reports. The caller placed the name, which is current.
pub(super) fn value(p: &mut Parser<'_>) {
    let column = p.current_column();
    if at_signature(p) {
        let m = p.start();
        value_name(p);
        expect_indented(p, K::Colon);
        type_(p);
        m.complete(p, K::Signature);
        // elm wants the two names to match; validation checks.
        if !(at_value_name(p) && p.current_column() == column) || at_signature(p) {
            return;
        }
        p.anchor();
    }
    definition(p);
}

/// At a value's name and the colon after it.
fn at_signature(p: &mut Parser<'_>) -> bool {
    let name_tokens = if p.at(K::LowerIdent) { 1 } else { 3 };
    p.nth_at_symbol(name_tokens, K::Colon)
}

fn definition(p: &mut Parser<'_>) {
    let m = p.start();
    value_name(p);
    while at_pattern_argument(p) {
        pattern_atom(p);
    }
    if expect_part(p, K::Eq) {
        expr(p);
    }
    m.complete(p, K::Definition);
}
