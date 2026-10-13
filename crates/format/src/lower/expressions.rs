//! Expressions, after upstream's `Parse/Expression.hs`.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxNode, SyntaxToken, significant_tokens};

use super::cursor::Cursor;
use super::definitions::{destructure, value_parts};
use super::literal::{is_number, literal, negative};
use super::patterns::pattern;
use super::{
    VALID, bool_literal, commas, eol_after, lower_name, namespace, pair, preceded, sectioned,
    start, surround, symbol, upper_name, with_eol,
};
use crate::tree::{
    ApplicationMultiline, Around, BinopsClause, C2, CaseBranch, Comments, CommonDeclaration, Expr,
    ForceMultiline, IfClause, LetDeclaration, LiteralValue, Lower, Multiline, Ref, c,
};

pub(super) fn expr(cx: &mut Cursor, node: &ast::Expr) -> Expr {
    nomi_base::grow_large(|| expr_unguarded(cx, node))
}

fn expr_unguarded(cx: &mut Cursor, node: &ast::Expr) -> Expr {
    match node {
        ast::Expr::LiteralExpr(e) => Expr::Literal(literal(e.syntax())),
        ast::Expr::NameExpr(e) => name(e),
        ast::Expr::OperatorFunction(e) => Expr::Var(Ref::Op(symbol(&e.op().expect(VALID)))),
        ast::Expr::TupleConstructorExpr(e) => Expr::TupleFunction(commas(e.syntax()).len() + 1),
        ast::Expr::NegateExpr(e) => {
            let operand = e.expr().expect(VALID);
            // Upstream reads a `-` touching a number as part of it.
            match &operand {
                ast::Expr::LiteralExpr(n)
                    if is_number(significant_tokens(n.syntax()).next().expect(VALID).kind()) =>
                {
                    Expr::Literal(negative(literal(n.syntax())))
                }
                _ => Expr::Negate(Box::new(expr(cx, &operand))),
            }
        }
        ast::Expr::CallExpr(e) => application(cx, e),
        ast::Expr::OperatorChain(e) => binops(cx, e),
        ast::Expr::UnitExpr(e) => Expr::Unit(cx.take(start(&e.r_paren_token().expect(VALID)))),
        ast::Expr::ParenExpr(e) => {
            let close = e.r_paren_token().expect(VALID);
            let items = e.expr().into_iter();
            let item = surround(cx, e.syntax(), items, &close, |cx, e| with_eol(cx, e, expr))
                .pop()
                .expect(VALID);
            Expr::Parens(Box::new(eol_after(item)))
        }
        ast::Expr::TupleExpr(e) => {
            let close = e.r_paren_token().expect(VALID);
            let items = surround(cx, e.syntax(), e.exprs(), &close, |cx, e| {
                with_eol(cx, e, expr)
            })
            .into_iter()
            .map(eol_after)
            .collect();
            Expr::Tuple(items, cx.force_multiline(e.syntax()))
        }
        ast::Expr::ListExpr(e) => {
            let close = e.r_bracket_token().expect(VALID);
            let (terms, trailing) = sectioned(cx, e.syntax(), e.exprs(), &close, expr);
            Expr::List {
                terms,
                trailing,
                force_multiline: cx.force_multiline(e.syntax()),
            }
        }
        ast::Expr::RangeExpr(e) => range(cx, e),
        ast::Expr::RecordExpr(e) => {
            let close = e.r_brace_token().expect(VALID);
            record(cx, e.syntax(), None, e.fields(), &close)
        }
        ast::Expr::RecordUpdateExpr(e) => {
            let name = e.lower_ident_token().expect(VALID);
            let before = cx.take(start(&name));
            let after = cx.take(start(&e.pipe_token().expect(VALID)));
            let base = c(Around { before, after }, lower_name(&name));
            let close = e.r_brace_token().expect(VALID);
            record(cx, e.syntax(), Some(base), e.fields(), &close)
        }
        ast::Expr::FieldAccessExpr(e) => {
            let record = expr(cx, &e.expr().expect(VALID));
            Expr::Access(
                Box::new(record),
                lower_name(&e.lower_ident_token().expect(VALID)),
            )
        }
        ast::Expr::AccessorExpr(e) => {
            Expr::AccessFunction(lower_name(&e.lower_ident_token().expect(VALID)))
        }
        ast::Expr::IfExpr(e) => if_expr(cx, e),
        ast::Expr::CaseExpr(e) => case(cx, e),
        ast::Expr::LetExpr(e) => let_expr(cx, e),
        ast::Expr::LambdaExpr(e) => lambda(cx, e),
        ast::Expr::GlslExpr(e) => glsl(e),
    }
}

/// A name; an unqualified `True` or `False` is a literal.
fn name(node: &ast::NameExpr) -> Expr {
    let namespace = namespace(node.qualifier());
    if let Some(name) = node.lower_ident_token() {
        return Expr::Var(Ref::Var(namespace, lower_name(&name)));
    }
    let name = upper_name(&node.upper_ident_token().expect(VALID));
    match bool_literal(&namespace, &name) {
        Some(value) => Expr::Literal(LiteralValue::Bool(value)),
        None => Expr::Var(Ref::Tag(namespace, name)),
    }
}

/// `[low..high]`.
fn range(cx: &mut Cursor, node: &ast::RangeExpr) -> Expr {
    let mut bounds = node.exprs();
    let (low, high) = (bounds.next().expect(VALID), bounds.next().expect(VALID));
    let low = bounded(cx, &low, start(&node.dot_dot_token().expect(VALID)));
    let high = bounded(cx, &high, start(&node.r_bracket_token().expect(VALID)));
    Expr::Range {
        low: Box::new(low),
        high: Box::new(high),
    }
}

/// `\args -> body`, with the comments before `->` and after it as one.
fn lambda(cx: &mut Cursor, node: &ast::LambdaExpr) -> Expr {
    let args = preceded(cx, node.patterns(), pattern);
    let body = node.expr().expect(VALID);
    let mut before_body = cx.take(start(&node.arrow_token().expect(VALID)));
    before_body.extend(cx.take(cx.range(body.syntax()).start()));
    Expr::Lambda {
        args,
        before_body,
        body: Box::new(expr(cx, &body)),
        multiline: cx.force_multiline(node.syntax()),
    }
}

/// `[glsl| … |]`, without the carriage returns.
fn glsl(node: &ast::GlslExpr) -> Expr {
    let token = node.glsl_token().expect(VALID);
    let text = token.text();
    let source = text["[glsl|".len()..text.len() - "|]".len()]
        .chars()
        .filter(|&c| c != '\r')
        .collect();
    Expr::GlShader(source)
}

/// `node`, with the comments before it and those before `end`.
fn bounded(cx: &mut Cursor, node: &ast::Expr, end: TextSize) -> C2<Expr> {
    let before = cx.take(cx.range(node.syntax()).start());
    let value = expr(cx, node);
    let after = cx.take(end);
    c(Around { before, after }, value)
}

/// A function and its arguments, breaking as the source does; upstream's
/// `appExpr`.
fn application(cx: &mut Cursor, node: &ast::CallExpr) -> Expr {
    let function = node.function().expect(VALID);
    let function_range = cx.range(function.syntax());
    let function_multiline = cx.multiline(function_range);
    let function = expr(cx, &function);
    // Each argument spans lines with the space before it.
    let mut end = function_range.end();
    let mut first_multiline = false;
    let mut rest_multiline = false;
    let mut args = Vec::new();
    for (i, arg) in node.arguments().enumerate() {
        let arg_range = cx.range(arg.syntax());
        let multiline = cx.multiline(TextRange::new(end, arg_range.end()));
        end = arg_range.end();
        if i == 0 {
            first_multiline = multiline;
        } else {
            rest_multiline |= multiline;
        }
        let comments = cx.take(arg_range.start());
        args.push(c(comments, expr(cx, &arg)));
    }
    let multiline = match (function_multiline || first_multiline, rest_multiline) {
        (true, _) => ApplicationMultiline::SplitFirst,
        (false, true) => ApplicationMultiline::JoinFirst(Multiline::SplitAll),
        (false, false) => ApplicationMultiline::JoinFirst(Multiline::JoinAll),
    };
    Expr::App {
        function: Box::new(function),
        args,
        multiline,
    }
}

/// `a + b * c`, flat as upstream reads it; fixities apply later.
fn binops(cx: &mut Cursor, node: &ast::OperatorChain) -> Expr {
    let mut operands = node.exprs();
    let first = expr(cx, &operands.next().expect(VALID));
    let clauses = node
        .ops()
        .zip(operands)
        .map(|(op, operand)| {
            let before_op = cx.take(cx.range(op.syntax()).start());
            let after_op = cx.take(cx.range(operand.syntax()).start());
            BinopsClause {
                before_op,
                op: Ref::Op(symbol(&op)),
                after_op,
                expr: expr(cx, &operand),
            }
        })
        .collect();
    Expr::Binops {
        first: Box::new(first),
        clauses,
        multiline: cx.force_multiline(node.syntax()),
    }
}

/// A record, or with `base` an update of one.
fn record(
    cx: &mut Cursor,
    node: &SyntaxNode,
    base: Option<C2<Lower>>,
    fields: impl Iterator<Item = ast::Field>,
    close: &SyntaxToken,
) -> Expr {
    let (fields, trailing) = sectioned(cx, node, fields, close, |cx, field| {
        pair(cx, field.syntax(), &field.expr().expect(VALID), expr)
    });
    Expr::Record {
        base,
        fields,
        trailing,
        force_multiline: cx.force_multiline(node),
    }
}

/// `if … then … else if … else …`: an `else` holding an `if` continues
/// the chain.
fn if_expr(cx: &mut Cursor, node: &ast::IfExpr) -> Expr {
    let first = if_clause(cx, node);
    let mut else_ifs = Vec::new();
    let mut link = node.clone();
    loop {
        match link.else_branch().expect(VALID) {
            ast::Expr::IfExpr(next) => {
                let comments = cx.take(start(&next.if_token().expect(VALID)));
                else_ifs.push(c(comments, if_clause(cx, &next)));
                link = next;
            }
            last => {
                let comments = cx.take(cx.range(last.syntax()).start());
                let body = expr(cx, &last);
                return Expr::If {
                    first: Box::new(first),
                    else_ifs,
                    else_body: Box::new(c(comments, body)),
                };
            }
        }
    }
}

/// `if condition then body`, up to the `else`.
fn if_clause(cx: &mut Cursor, node: &ast::IfExpr) -> IfClause {
    let condition = node.condition().expect(VALID);
    let condition = bounded(cx, &condition, start(&node.then_token().expect(VALID)));
    let body = node.then_branch().expect(VALID);
    let body = bounded(cx, &body, start(&node.else_token().expect(VALID)));
    IfClause { condition, body }
}

fn case(cx: &mut Cursor, node: &ast::CaseExpr) -> Expr {
    // Upstream reads `then` for `of`.
    let of = node.of_token().or_else(|| node.then_token()).expect(VALID);
    let subject = bounded(cx, &node.expr().expect(VALID), start(&of));
    // Upstream tracks the newlines around the subject too.
    let after_case = node.case_token().expect(VALID).text_range().end();
    let multiline = ForceMultiline(cx.multiline(TextRange::new(after_case, start(&of))));
    let branches = node
        .case_branches()
        .map(|branch| {
            let target = branch.pattern().expect(VALID);
            let before_pattern = cx.take(cx.range(target.syntax()).start());
            let target = pattern(cx, &target);
            let before_arrow = cx.take(start(&branch.arrow_token().expect(VALID)));
            let body = branch.expr().expect(VALID);
            let after_arrow = cx.take(cx.range(body.syntax()).start());
            CaseBranch {
                before_pattern,
                before_arrow,
                after_arrow,
                pattern: target,
                body: expr(cx, &body),
            }
        })
        .collect();
    Expr::Case {
        subject: Box::new(subject),
        multiline,
        branches,
    }
}

/// `let … in body`: upstream keeps each comment between declarations as
/// one of them, and an annotation apart from its definition.
fn let_expr(cx: &mut Cursor, node: &ast::LetExpr) -> Expr {
    let mut declarations = Vec::new();
    let mut push = |comments: Comments, declaration: CommonDeclaration| {
        declarations.extend(comments.into_iter().map(LetDeclaration::Comment));
        declarations.push(LetDeclaration::Common(Box::new(declaration)));
    };
    for declaration in node.let_decls() {
        match declaration {
            ast::LetDecl::LetValue(value) => {
                value_parts(cx, value.signature(), value.definition(), &mut push);
            }
            ast::LetDecl::LetDestructure(value) => {
                let comments = cx.take(cx.range(value.syntax()).start());
                let lowered = destructure(
                    cx,
                    &value.pattern().expect(VALID),
                    &value.eq_token().expect(VALID),
                    &value.expr().expect(VALID),
                );
                push(comments, lowered);
            }
        }
    }
    let comments = cx.take(start(&node.in_token().expect(VALID)));
    declarations.extend(comments.into_iter().map(LetDeclaration::Comment));
    let body = node.expr().expect(VALID);
    let before_body = cx.take(cx.range(body.syntax()).start());
    Expr::Let {
        declarations,
        before_body,
        body: Box::new(expr(cx, &body)),
    }
}

#[cfg(test)]
mod tests;
