//! Expressions, such as `List.map .name model.users ++ [ -1 ]`. Operator
//! chains stay flat; precedence is resolved after canonicalization.

use super::definitions::value;
use super::patterns::{at_pattern_atom, pattern, pattern_atom};
use super::{at_aligned, expect_indented, expect_part, qualified_name, skip_junk};
use crate::SyntaxKind as K;
use crate::parser::{CompletedMarker, Parser, SyntaxErrorKind};

/// A whole expression.
pub(super) fn expr(p: &mut Parser<'_>) {
    p.guarded(|p| {
        if at_compound(p) {
            compound(p);
        } else {
            chain(p);
        }
    });
}

/// `a + f x - 1`: calls joined by operators, in one flat node. An `if`,
/// `case`, `let` or lambda may come last: `a + \x -> x`.
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
        if at_compound(p) {
            compound(p);
            break;
        }
        call(p);
    }
    m.complete(p, K::OperatorChain);
}

/// Whether an expression that runs as far right as it can starts here.
fn at_compound(p: &mut Parser<'_>) -> bool {
    matches!(p.current(), K::IfKw | K::CaseKw | K::LetKw | K::Backslash) && p.at_indented()
}

fn compound(p: &mut Parser<'_>) -> CompletedMarker {
    match p.current() {
        K::IfKw => if_expr(p),
        K::CaseKw => case_expr(p),
        K::LetKw => let_expr(p),
        K::Backslash => lambda(p),
        _ => unreachable!("at_compound checked the token"),
    }
}

/// `if a then b else c`. An `else if` nests in the `else` branch.
fn if_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    expr(p);
    if expect_part(p, K::ThenKw) {
        expr(p);
        if expect_part(p, K::ElseKw) {
            expr(p);
        }
    }
    m.complete(p, K::IfExpr)
}

/// `case e of` and its branches, aligned with the first.
fn case_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    expr(p);
    if expect_part(p, K::OfKw) {
        if at_pattern_atom(p) {
            let column = p.current_column();
            p.with_min_column(column + 1, |p| {
                p.anchor();
                loop {
                    case_branch(p);
                    if !at_aligned(p, column, at_pattern_atom) {
                        break;
                    }
                }
            });
        } else {
            p.error(SyntaxErrorKind::ExpectedPattern);
        }
    }
    m.complete(p, K::CaseExpr)
}

fn case_branch(p: &mut Parser<'_>) {
    let m = p.start();
    pattern(p);
    if expect_part(p, K::Arrow) {
        expr(p);
    }
    m.complete(p, K::CaseBranch);
}

/// `let`, definitions aligned with the first, `in` and the body. The
/// definitions must be right of the `let`, not just of the enclosing
/// construct.
fn let_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let let_column = p.current_column();
    p.bump();
    if at_pattern_atom(p) {
        let column = p.current_column();
        if column <= let_column {
            p.error(SyntaxErrorKind::ExpectedIndent);
        }
        p.with_min_column(column + 1, |p| {
            p.anchor();
            loop {
                let_decl(p);
                if !at_aligned(p, column, at_pattern_atom) {
                    break;
                }
            }
        });
    } else {
        p.error(SyntaxErrorKind::Expected(K::Definition));
    }
    if expect_part(p, K::InKw) {
        expr(p);
    }
    m.complete(p, K::LetExpr)
}

/// `name args = body`, maybe with a signature, or `( a, b ) = pair`.
fn let_decl(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(K::LowerIdent) {
        value(p);
        m.complete(p, K::LetValue);
    } else {
        pattern_atom(p);
        if expect_part(p, K::Eq) {
            expr(p);
        }
        m.complete(p, K::LetDestructure);
    }
}

/// `\x y -> body`.
fn lambda(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    if at_pattern_atom(p) {
        while at_pattern_atom(p) {
            pattern_atom(p);
        }
    } else {
        p.error(SyntaxErrorKind::ExpectedPattern);
        p.missing();
    }
    if expect_part(p, K::Arrow) {
        expr(p);
    }
    m.complete(p, K::LambdaExpr)
}

fn at_operator(p: &mut Parser<'_>) -> bool {
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
fn at_negated(p: &mut Parser<'_>) -> bool {
    p.nth_is_joined(1) && starts_atom(p.nth(1))
}

/// An argument, including `-x` after space, which elm reads as a negated
/// argument rather than a subtraction: `f -1` is `f (-1)`.
fn at_argument(p: &mut Parser<'_>) -> bool {
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
fn at_atom(p: &mut Parser<'_>) -> bool {
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
fn at_tuple_constructor(p: &mut Parser<'_>) -> bool {
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
    if p.at(K::LowerIdent) && p.nth_at_symbol(1, K::Pipe) {
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
    if p.at_symbol(K::Colon) {
        p.check_indented();
        p.bump_symbol(K::Colon);
    } else {
        expect_indented(p, K::Eq);
    }
    expr(p);
    m.complete(p, K::Field);
}

#[cfg(test)]
mod tests;
