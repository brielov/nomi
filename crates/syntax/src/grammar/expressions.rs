//! Expressions, such as `List.map .name model.users ++ [ -1 ]`. Operator
//! chains stay flat; precedence is resolved after canonicalization.

use super::{expect_indented, qualified_name, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, SyntaxErrorKind};

/// A whole expression.
pub(super) fn expr(p: &mut Parser<'_>) {
    p.guarded(chain);
}

/// `a + f x - 1`: calls joined by operators, in one flat node.
fn chain(p: &mut Parser<'_>) {
    let first = call(p);
    if !at_operator(p) {
        return;
    }
    let m = first.precede(p);
    while at_operator(p) {
        let op = p.start();
        p.bump();
        op.complete(p, K::Op);
        call(p);
    }
    m.complete(p, K::OperatorChain);
}

fn at_operator(p: &Parser<'_>) -> bool {
    matches!(p.current(), K::Operator | K::Minus | K::ColonColon) && p.at_indented()
}

/// A function applied to its arguments, or a lone operand.
fn call(p: &mut Parser<'_>) -> CompletedMarker {
    let head = operand(p);
    if !at_argument(p) {
        return head;
    }
    let m = head.precede(p);
    while at_argument(p) {
        if p.at(K::Minus) {
            negate(p);
        } else {
            atom(p);
        }
    }
    m.complete(p, K::CallExpr)
}

/// An atom, or one negated by a `-` touching it.
fn operand(p: &mut Parser<'_>) -> CompletedMarker {
    if p.at(K::Minus) && p.at_indented() && at_negated(p) {
        return negate(p);
    }
    if !at_atom(p) {
        p.error(SyntaxErrorKind::ExpectedExpression);
        return p.missing();
    }
    atom(p)
}

/// Whether the `-` here touches an atom.
fn at_negated(p: &Parser<'_>) -> bool {
    p.nth_is_joined(1) && starts_atom(p.nth(1))
}

/// An argument, including `-x` after space, which elm reads as a negated
/// argument rather than a subtraction: `f -1` is `f (-1)`.
fn at_argument(p: &Parser<'_>) -> bool {
    if p.at(K::Minus) {
        p.at_indented() && !p.nth_is_joined(0) && at_negated(p)
    } else {
        at_atom(p)
    }
}

fn negate(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    atom(p);
    m.complete(p, K::NegateExpr)
}

/// Whether an atom starts here and continues the construct being parsed.
fn at_atom(p: &Parser<'_>) -> bool {
    let starts = if p.at(K::Dot) {
        p.nth(1) == K::LowerIdent && p.nth_is_joined(1)
    } else {
        starts_atom(p.current())
    };
    starts && p.at_indented()
}

/// Whether `kind` starts an atom, a `.field` accessor aside.
fn starts_atom(kind: K) -> bool {
    matches!(
        kind,
        K::LowerIdent
            | K::UpperIdent
            | K::Int
            | K::Float
            | K::Char
            | K::String
            | K::MultilineString
            | K::Glsl
            | K::LParen
            | K::LBracket
            | K::LBrace
    )
}

/// An expression that needs no parentheses as an argument, with any
/// field accesses. The caller checks [`at_atom`] or [`at_negated`].
fn atom(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let (kind, accessible) = match p.current() {
        K::LowerIdent | K::UpperIdent => {
            qualified_name(p, &[K::UpperIdent, K::LowerIdent]);
            (K::NameExpr, true)
        }
        K::Int | K::Float | K::Char | K::String | K::MultilineString => {
            p.bump();
            (K::LiteralExpr, false)
        }
        K::Glsl => {
            p.bump();
            (K::GlslExpr, false)
        }
        K::Dot => {
            p.bump();
            p.bump();
            (K::AccessorExpr, false)
        }
        // As in elm, anything in parentheses takes accesses, even `(+)`.
        K::LParen => (paren(p), true),
        K::LBracket => (list(p), false),
        K::LBrace => (record(p), true),
        _ => unreachable!("the caller checked the token"),
    };
    let atom = m.complete(p, kind);
    if accessible {
        field_access(p, atom)
    } else {
        atom
    }
}

/// `.name` touching `target` and each other: `model.user.name`. Each
/// access nests, so each is guarded.
fn field_access(p: &mut Parser<'_>, target: CompletedMarker) -> CompletedMarker {
    let at_access =
        p.at(K::Dot) && p.nth_is_joined(0) && p.nth(1) == K::LowerIdent && p.nth_is_joined(1);
    if !at_access {
        return target;
    }
    let m = target.precede(p);
    p.bump();
    p.bump();
    let access = m.complete(p, K::FieldAccessExpr);
    p.guarded(|p| field_access(p, access)).unwrap_or(access)
}

/// `()`, `(+)`, `(,)`, `(e)` or `(a, b)`. Returns the kind for the
/// caller's node.
// TODO(plan 1.15): elm wants the closing parenthesis indented.
fn paren(p: &mut Parser<'_>) -> K {
    p.bump();
    if p.at(K::RParen) {
        p.check_indented();
        p.bump();
        return K::UnitExpr;
    }
    if matches!(p.current(), K::Operator | K::Minus | K::ColonColon) && p.nth(1) == K::RParen {
        p.check_indented();
        let op = p.start();
        p.bump();
        op.complete(p, K::Op);
        p.bump();
        return K::OperatorFunction;
    }
    if p.at(K::Comma) && at_tuple_constructor(p) {
        while p.at(K::Comma) {
            p.check_indented();
            p.bump();
        }
        p.bump();
        return K::TupleConstructorExpr;
    }
    expr(p);
    let mut kind = K::ParenExpr;
    loop {
        if p.eat(K::RParen) {
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            kind = K::TupleExpr;
            expr(p);
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

/// `,` up to `)`, after the `(`.
fn at_tuple_constructor(p: &Parser<'_>) -> bool {
    let mut n = 0;
    while p.nth(n) == K::Comma {
        n += 1;
    }
    p.nth(n) == K::RParen
}

/// `[]` or `[a, b]`.
fn list(p: &mut Parser<'_>) -> K {
    p.bump();
    if !p.at(K::RBracket) {
        expr(p);
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
            expr(p);
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
    K::ListExpr
}

/// `{}`, `{ a = 1 }` or `{ r | a = 1 }`.
fn record(p: &mut Parser<'_>) -> K {
    p.bump();
    if p.at(K::RBrace) {
        p.check_indented();
        p.bump();
        return K::RecordExpr;
    }
    let mut kind = K::RecordExpr;
    if p.at(K::LowerIdent) && p.nth(1) == K::Pipe {
        p.check_indented();
        p.bump();
        expect_indented(p, K::Pipe);
        kind = K::RecordUpdateExpr;
    }
    field(p);
    loop {
        if p.at(K::RBrace) {
            p.check_indented();
            p.bump();
            break;
        }
        if p.at(K::Comma) {
            p.check_indented();
            p.bump();
            field(p);
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
    kind
}

/// `name = value`, or elm-format's `name : value`.
fn field(p: &mut Parser<'_>) {
    if !p.at(K::LowerIdent) {
        p.error(SyntaxErrorKind::Expected(K::LowerIdent));
        p.missing();
        return;
    }
    let m = p.start();
    p.check_indented();
    p.bump();
    if p.at(K::Colon) {
        p.check_indented();
        p.bump();
    } else {
        expect_indented(p, K::Eq);
    }
    expr(p);
    m.complete(p, K::Field);
}

#[cfg(test)]
mod tests;
