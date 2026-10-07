//! Value definitions with an optional signature, shared by let blocks and
//! the top level.

use super::expressions::expr;
use super::patterns::{at_pattern_atom, pattern_atom};
use super::types::type_;
use super::{expect_indented, expect_part};
use crate::SyntaxKind as K;
use crate::parser::{Parser, SyntaxErrorKind};

/// `name : Type` on one line and `name args = body` aligned below it, or
/// just the definition. The caller placed the name, which is current.
pub(super) fn value(p: &mut Parser<'_>) {
    let column = p.current_column();
    if p.nth_at_symbol(1, K::Colon) {
        let m = p.start();
        p.check_indented();
        p.bump();
        expect_indented(p, K::Colon);
        type_(p);
        m.complete(p, K::Signature);
        // elm wants the two names to match; validation checks.
        if !(p.at(K::LowerIdent) && p.current_column() == column) {
            p.error(SyntaxErrorKind::Expected(K::Definition));
            return;
        }
        p.anchor();
    }
    definition(p);
}

fn definition(p: &mut Parser<'_>) {
    let m = p.start();
    p.check_indented();
    p.bump();
    while at_pattern_atom(p) {
        pattern_atom(p);
    }
    if expect_part(p, K::Eq) {
        expr(p);
    }
    m.complete(p, K::Definition);
}
