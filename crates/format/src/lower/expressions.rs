//! Expressions, after upstream's `Parse/Expression.hs`.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use super::cursor::Cursor;
use super::definitions::{annotation, definition, destructure};
use super::literal::{is_number, literal, negative};
use super::patterns::pattern;
use super::{
    VALID, c, commas, lower_name, namespace, pair, sectioned, start, surround, symbol, token,
    upper_name,
};
use crate::tree::{
    ApplicationMultiline, BinopsClause, C0Eol, C2, CaseBranch, Comment, Expr, ForceMultiline,
    IfClause, LetDeclaration, LiteralValue, Lower, Multiline, Ref, UnaryOperator,
};

pub(super) fn expr(cx: &mut Cursor, node: &ast::Expr) -> Expr {
    crate::guarded(|| expr_unguarded(cx, node))
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
                ast::Expr::LiteralExpr(n) if is_number(n.syntax()) => {
                    Expr::Literal(negative(literal(n.syntax())))
                }
                _ => Expr::Unary(UnaryOperator::Negative, Box::new(expr(cx, &operand))),
            }
        }
        ast::Expr::CallExpr(e) => application(cx, e),
        ast::Expr::OperatorChain(e) => binops(cx, e),
        ast::Expr::UnitExpr(e) => Expr::Unit(cx.take(start(&e.r_paren_token().expect(VALID)))),
        ast::Expr::ParenExpr(e) => {
            let close = e.r_paren_token().expect(VALID);
            let item = surround(cx, e.syntax(), e.expr().into_iter(), &close, with_eol)
                .pop()
                .expect(VALID);
            Expr::Parens(Box::new(eol_after(item)))
        }
        ast::Expr::TupleExpr(e) => {
            let close = e.r_paren_token().expect(VALID);
            let items = surround(cx, e.syntax(), e.exprs(), &close, with_eol)
                .into_iter()
                .map(eol_after)
                .collect();
            Expr::Tuple(items, cx.multiline(cx.range(e.syntax())))
        }
        ast::Expr::ListExpr(e) => {
            let close = e.r_bracket_token().expect(VALID);
            let (terms, trailing) = sectioned(cx, e.syntax(), e.exprs(), &close, expr);
            Expr::List {
                terms,
                trailing,
                force_multiline: ForceMultiline(cx.multiline(cx.range(e.syntax()))),
            }
        }
        ast::Expr::RangeExpr(e) => {
            let mut bounds = e.exprs();
            let (low, high) = (bounds.next().expect(VALID), bounds.next().expect(VALID));
            let dots = e.dot_dot_token().expect(VALID);
            let close = e.r_bracket_token().expect(VALID);
            let low = bounded(cx, &low, start(&dots));
            let high = bounded(cx, &high, start(&close));
            Expr::Range(
                Box::new(low),
                Box::new(high),
                cx.multiline(cx.range(e.syntax())),
            )
        }
        ast::Expr::RecordExpr(e) => {
            let close = e.r_brace_token().expect(VALID);
            record(cx, e.syntax(), None, e.fields(), &close)
        }
        ast::Expr::RecordUpdateExpr(e) => {
            let name = e.lower_ident_token().expect(VALID);
            let pre = cx.take(start(&name));
            let post = cx.take(start(&e.pipe_token().expect(VALID)));
            let base = c((pre, post), lower_name(&name));
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
        ast::Expr::LambdaExpr(e) => {
            let args = e
                .patterns()
                .map(|arg| {
                    let comments = cx.take(cx.range(arg.syntax()).start());
                    c(comments, pattern(cx, &arg))
                })
                .collect();
            let body = e.expr().expect(VALID);
            let mut comments = cx.take(start(&e.arrow_token().expect(VALID)));
            comments.extend(cx.take(cx.range(body.syntax()).start()));
            let body = expr(cx, &body);
            Expr::Lambda(
                args,
                comments,
                Box::new(body),
                cx.multiline(cx.range(e.syntax())),
            )
        }
        ast::Expr::GlslExpr(e) => {
            let text = e.glsl_token().expect(VALID);
            let source = text.text()["[glsl|".len()..text.text().len() - "|]".len()]
                .chars()
                .filter(|&c| c != '\r')
                .collect();
            Expr::GlShader(source)
        }
    }
}

/// A name; an unqualified `True` or `False` is a literal.
fn name(node: &ast::NameExpr) -> Expr {
    let namespace = namespace(node.qualifier());
    if let Some(name) = token(node.syntax(), SyntaxKind::LowerIdent) {
        return Expr::Var(Ref::Var(namespace, lower_name(&name)));
    }
    let name = upper_name(&token(node.syntax(), SyntaxKind::UpperIdent).expect(VALID));
    if namespace.is_empty() {
        match name.0.as_str() {
            "True" => return Expr::Literal(LiteralValue::Bool(true)),
            "False" => return Expr::Literal(LiteralValue::Bool(false)),
            _ => {}
        }
    }
    Expr::Var(Ref::Tag(namespace, name))
}

/// An expression with the `--` comment ending its line, unless it spans
/// lines.
fn with_eol(cx: &mut Cursor, node: &ast::Expr) -> C0Eol<Expr> {
    let value = expr(cx, node);
    c(cx.eol(cx.range(node.syntax())), value)
}

/// An item of a parenthesized list, its `--` comment first among those
/// after it; upstream's `spaceySepBy1''`.
fn eol_after(item: C2<C0Eol<Expr>>) -> C2<Expr> {
    let ((pre, mut post), inner) = (item.comments, item.value);
    if let Some(eol) = inner.comments {
        post.insert(0, Comment::Line(eol));
    }
    c((pre, post), inner.value)
}

/// `node`, with the comments before it and those before `end`.
fn bounded(cx: &mut Cursor, node: &ast::Expr, end: TextSize) -> C2<Expr> {
    let pre = cx.take(cx.range(node.syntax()).start());
    let value = expr(cx, node);
    c((pre, cx.take(end)), value)
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
    Expr::App(Box::new(function), args, multiline)
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
    Expr::Binops(
        Box::new(first),
        clauses,
        cx.multiline(cx.range(node.syntax())),
    )
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
        force_multiline: ForceMultiline(cx.multiline(cx.range(node))),
    }
}

/// `if … then … else if … else …`: an `else` holding an `if` continues
/// the chain.
fn if_expr(cx: &mut Cursor, node: &ast::IfExpr) -> Expr {
    let first = if_clause(cx, node);
    let mut rest = Vec::new();
    let mut link = node.clone();
    loop {
        match link.else_branch().expect(VALID) {
            ast::Expr::IfExpr(next) => {
                let comments = cx.take(start(&next.if_token().expect(VALID)));
                rest.push(c(comments, if_clause(cx, &next)));
                link = next;
            }
            last => {
                let comments = cx.take(cx.range(last.syntax()).start());
                let body = expr(cx, &last);
                return Expr::If(Box::new(first), rest, Box::new(c(comments, body)));
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
    let of = [SyntaxKind::OfKw, SyntaxKind::ThenKw]
        .into_iter()
        .find_map(|kind| token(node.syntax(), kind))
        .expect(VALID);
    let subject = bounded(cx, &node.expr().expect(VALID), start(&of));
    // Upstream tracks the newlines around the subject too.
    let after_case = node.case_token().expect(VALID).text_range().end();
    let multiline = cx.multiline(TextRange::new(after_case, start(&of)));
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
    Expr::Case(Box::new(subject), multiline, branches)
}

/// `let … in body`: upstream keeps each comment between declarations as
/// one of them, and an annotation apart from its definition.
fn let_expr(cx: &mut Cursor, node: &ast::LetExpr) -> Expr {
    let mut declarations = Vec::new();
    let comments = |cx: &mut Cursor, end, declarations: &mut Vec<LetDeclaration>| {
        let taken = cx.take(end);
        declarations.extend(taken.into_iter().map(LetDeclaration::Comment));
    };
    for declaration in node.let_decls() {
        match declaration {
            ast::LetDecl::LetValue(value) => {
                if let Some(signature) = value.signature() {
                    comments(cx, cx.range(signature.syntax()).start(), &mut declarations);
                    let lowered = annotation(cx, &signature);
                    declarations.push(LetDeclaration::Common(Box::new(lowered)));
                }
                if let Some(def) = value.definition() {
                    comments(cx, cx.range(def.syntax()).start(), &mut declarations);
                    let lowered = definition(cx, &def);
                    declarations.push(LetDeclaration::Common(Box::new(lowered)));
                }
            }
            ast::LetDecl::LetDestructure(value) => {
                comments(cx, cx.range(value.syntax()).start(), &mut declarations);
                let lowered = destructure(
                    cx,
                    &value.pattern().expect(VALID),
                    &value.eq_token().expect(VALID),
                    &value.expr().expect(VALID),
                );
                declarations.push(LetDeclaration::Common(Box::new(lowered)));
            }
        }
    }
    comments(cx, start(&node.in_token().expect(VALID)), &mut declarations);
    let body = node.expr().expect(VALID);
    let before_body = cx.take(cx.range(body.syntax()).start());
    Expr::Let(declarations, before_body, Box::new(expr(cx, &body)))
}

#[cfg(test)]
mod tests;
