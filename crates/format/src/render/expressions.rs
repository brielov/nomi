//! Expressions; upstream's `formatExpression` and the helpers only it
//! uses.

use crate::block::{Block, Line};
use crate::structure::{application, group};
use crate::tree::{
    ApplicationMultiline, BinopsClause, C1, C2, CaseBranch, Comment, Commented, CommonDeclaration,
    Expr, IfClause, LetDeclaration, LiteralValue, Lower, Multiline, Pattern, Ref, Upper,
};

use super::SyntaxContext::{self, AmbiguousEnd, InfixSeparated, SpaceSeparated, SyntaxSeparated};
use super::declarations::common;
use super::patterns::{format_pattern, pattern_argument};
use super::{
    BinaryOp, binary, comment, commented, commented_forced, literal::literal, name, pair, parens,
    pre_commented, qualified, record_like, sequence, spaced, syntax_parens, unit,
};

/// `expr` where nothing around it calls for parentheses.
pub(super) fn expression_alone(expr: &Expr) -> Block {
    syntax_parens(SyntaxSeparated, format_expression(expr))
}

fn text(text: impl Into<String>) -> Block {
    Block::line(Line::text(text))
}

/// A name in `namespace`; upstream's `formatLowercaseIdentifier` and the
/// `TagRef` case of `formatVar`.
fn qualified_name(namespace: &[Upper], last: &str) -> Line {
    if namespace.is_empty() {
        return name(last);
    }
    Line::row([qualified(namespace), Line::text("."), name(last)])
}

/// Upstream's `formatVar`.
fn var(var: &Ref) -> Line {
    match var {
        Ref::Var(namespace, Lower(last)) => qualified_name(namespace, last),
        Ref::Tag(namespace, Upper(last)) => qualified_name(namespace, last),
        Ref::Op(op) => Line::text(format!("({})", op.0)),
    }
}

/// Upstream's `formatInfixVar`: an operator as it is, a name in
/// backquotes.
fn infix_var(op: &Ref) -> Line {
    match op {
        Ref::Op(op) => Line::text(op.0.as_str()),
        named => Line::row([Line::text("`"), var(named), Line::text("`")]),
    }
}

/// Upstream's `formatExpression`.
pub(super) fn format_expression(expr: &Expr) -> (SyntaxContext, Block) {
    crate::guarded(|| format_expression_unguarded(expr))
}

fn format_expression_unguarded(expr: &Expr) -> (SyntaxContext, Block) {
    match expr {
        Expr::Literal(value) => (SyntaxSeparated, literal(value)),
        Expr::Var(v) => (SyntaxSeparated, Block::line(var(v))),
        Expr::Range(left, right, _) => range(left, right),
        Expr::List {
            terms,
            trailing,
            force_multiline,
        } => (
            SyntaxSeparated,
            sequence(
                '[',
                ',',
                Some(']'),
                *force_multiline,
                trailing,
                terms,
                expression_alone,
            ),
        ),
        Expr::Binops(left, ops, multiline) => (InfixSeparated, binops(left, ops, *multiline)),
        Expr::Lambda(patterns, body_comments, body, multiline) => (
            AmbiguousEnd,
            lambda(patterns, body_comments, body, *multiline),
        ),
        Expr::Unary(_, e) => {
            let operand = format_expression(e);
            // Upstream prints `-(-x)` as `--x`, a comment; nomi keeps the
            // parentheses.
            let operand = if starts_with_minus(e) {
                parens(operand.1)
            } else {
                syntax_parens(SpaceSeparated, operand)
            };
            (SyntaxSeparated, operand.prefix(&Line::text("-")))
        }
        Expr::App(left, args, _) if args.is_empty() => format_expression(left),
        Expr::App(left, args, multiline) => (
            SpaceSeparated,
            application(
                *multiline,
                syntax_parens(InfixSeparated, format_expression(left)),
                args.iter()
                    .map(|a| pre_commented_expression(SpaceSeparated, a))
                    .collect(),
            ),
        ),
        Expr::If(first, else_ifs, els) => (AmbiguousEnd, if_(first, else_ifs, els)),
        Expr::Let(decls, body_comments, body) => (AmbiguousEnd, let_(decls, body_comments, body)),
        Expr::Case(subject, multiline, branches) => {
            (AmbiguousEnd, case(subject, *multiline, branches))
        }
        Expr::Tuple(exprs, multiline) => (
            SyntaxSeparated,
            group(
                true,
                "(",
                ",",
                ")",
                *multiline,
                exprs.iter().map(commented_expression).collect(),
            ),
        ),
        Expr::TupleFunction(n) => (SyntaxSeparated, text(format!("({})", ",".repeat(n - 1)))),
        Expr::Access(e, field) => (
            SyntaxSeparated,
            syntax_parens(SpaceSeparated, format_expression(e))
                .add_suffix(Line::row([Line::text("."), name(&field.0)])),
        ),
        Expr::AccessFunction(field) => (
            SyntaxSeparated,
            text(format!(".{}", field.0.replace('\'', "_"))),
        ),
        Expr::Record {
            base,
            fields,
            trailing,
            force_multiline,
        } => (
            SyntaxSeparated,
            record_like(base.as_ref(), fields, trailing, *force_multiline, |field| {
                pair("=", field, |key| name(&key.0), expression_alone)
            }),
        ),
        Expr::Parens(inner) => match &inner.comments {
            (pre, post) if pre.is_empty() && post.is_empty() => format_expression(&inner.value),
            _ => (SyntaxSeparated, parens(commented_expression(inner))),
        },
        Expr::Unit(comments) => (SyntaxSeparated, unit('(', ')', comments)),
        Expr::GlShader(source) => (
            SyntaxSeparated,
            Block::line(Line::row([
                Line::text("[glsl|"),
                Line::text(source.as_str()),
                Line::text("|]"),
            ])),
        ),
    }
}

/// Whether `expr` prints starting with a `-`.
fn starts_with_minus(expr: &Expr) -> bool {
    match expr {
        Expr::Unary(..) => true,
        Expr::Literal(LiteralValue::Int(n, _)) => *n < 0,
        Expr::Literal(LiteralValue::Float(f, _)) => f.is_sign_negative(),
        Expr::Parens(inner) if inner.comments.0.is_empty() && inner.comments.1.is_empty() => {
            starts_with_minus(&inner.value)
        }
        Expr::Access(record, _) => starts_with_minus(record),
        _ => false,
    }
}

/// Upstream's `formatCommentedExpression`: parentheses with comments
/// inside give theirs up to the slot around them.
fn commented_expression(expr: &C2<Expr>) -> Block {
    let (pre, post) = &expr.comments;
    match &expr.value {
        Expr::Parens(inner) => {
            let (inner_pre, inner_post) = &inner.comments;
            let pre = [&pre[..], inner_pre].concat();
            let post = [&inner_post[..], post].concat();
            commented(&pre, expression_alone(&inner.value), &post)
        }
        value => commented(pre, expression_alone(value), post),
    }
}

/// Upstream's `formatPreCommentedExpression`.
fn pre_commented_expression(context: SyntaxContext, expr: &C1<Expr>) -> Block {
    match &expr.value {
        Expr::Parens(inner) if inner.comments.1.is_empty() => {
            let pre = [&expr.comments[..], &inner.comments.0].concat();
            pre_commented(
                &pre,
                syntax_parens(context, format_expression(&inner.value)),
            )
        }
        value => pre_commented(
            &expr.comments,
            syntax_parens(context, format_expression(value)),
        ),
    }
}

/// `[a..b]` as Elm 0.19 spells it, `List.range a b`; upstream's
/// `formatRange_0_18`.
fn range(left: &C2<Expr>, right: &C2<Expr>) -> (SyntaxContext, Block) {
    let range = Box::new(Expr::Var(Ref::Var(
        vec![Upper("List".to_owned())],
        Lower("range".to_owned()),
    )));
    // Upstream parenthesizes both operands when either has comments after.
    let args = if left.comments.1.is_empty() && right.comments.1.is_empty() {
        [left, right]
            .map(|e| Commented {
                comments: e.comments.0.clone(),
                value: e.value.clone(),
            })
            .into()
    } else {
        [left, right]
            .map(|e| Commented {
                comments: Vec::new(),
                value: Expr::Parens(Box::new(e.clone())),
            })
            .into()
    };
    format_expression(&Expr::App(
        range,
        args,
        ApplicationMultiline::JoinFirst(Multiline::JoinAll),
    ))
}

/// Upstream's `formatBinops`.
fn binops(left: &Expr, ops: &[BinopsClause], multiline: bool) -> Block {
    let last = ops.len().saturating_sub(1);
    let ops = ops
        .iter()
        .enumerate()
        .map(|(i, clause)| {
            let left_pipe = matches!(&clause.op, Ref::Op(op) if op.0 == "<|");
            let context = if left_pipe && i == last {
                AmbiguousEnd
            } else {
                InfixSeparated
            };
            BinaryOp {
                left_pipe,
                comments: &clause.before_op,
                op: Block::line(infix_var(&clause.op)),
                next: pre_commented(
                    &clause.after_op,
                    syntax_parens(context, format_expression(&clause.expr)),
                ),
            }
        })
        .collect();
    binary(
        multiline,
        syntax_parens(InfixSeparated, format_expression(left)),
        ops,
    )
}

/// `\patterns -> body`.
fn lambda(
    patterns: &[C1<Pattern>],
    body_comments: &[Comment],
    body: &Expr,
    multiline: bool,
) -> Block {
    let patterns = patterns
        .iter()
        .map(|p| pre_commented(&p.comments, pattern_argument(&p.value)))
        .collect();
    let body = expression_alone(body);
    let indented_body =
        |body| Block::stack1(body_comments.iter().map(comment).chain([body])).indent();
    match (Block::all_singles(patterns), body) {
        (Ok(patterns), Block::SingleLine(body)) if !multiline && body_comments.is_empty() => {
            Block::line(Line::row([
                Line::text("\\"),
                Line::row(spaced(patterns)),
                Line::space(),
                Line::text("->"),
                Line::space(),
                body,
            ]))
        }
        (Ok(patterns), body) => Block::stack1([
            Block::line(Line::row([
                Line::text("\\"),
                Line::row(spaced(patterns)),
                Line::space(),
                Line::text("->"),
            ])),
            indented_body(body),
        ]),
        (Err(patterns), body) => Block::stack1([
            Block::stack1(patterns).prefix(&Line::text("\\")),
            text("->"),
            indented_body(body),
        ]),
    }
}

/// `if cond then`, on one line if it fits.
fn if_opening(key: Block, condition: Block) -> Block {
    match (key, condition) {
        (Block::SingleLine(key), Block::SingleLine(condition)) => Block::line(Line::row([
            key,
            Line::space(),
            condition,
            Line::space(),
            Line::text("then"),
        ])),
        (key, condition) => Block::stack1([key, condition.indent(), text("then")]),
    }
}

/// A clause's body, under its opening.
fn if_body(body: &C2<Expr>) -> Block {
    let (pre, post) = &body.comments;
    commented_forced(pre, expression_alone(&body.value), post).indent()
}

fn if_(first: &IfClause, else_ifs: &[C1<IfClause>], els: &C1<Expr>) -> Block {
    let opening = if_opening(text("if"), commented_expression(&first.condition));
    let mut block = Block::stack1([opening, if_body(&first.body)]);
    for else_if in else_ifs {
        let key = match pre_commented(&else_if.comments, text("if")) {
            Block::SingleLine(key) => {
                Block::line(Line::row([Line::text("else"), Line::space(), key]))
            }
            key => Block::stack1([text("else"), key]),
        };
        let clause = &else_if.value;
        block = block.and_then([
            Block::blank_line(),
            if_opening(key, commented_expression(&clause.condition)),
            if_body(&clause.body),
        ]);
    }
    block.and_then([
        Block::blank_line(),
        text("else"),
        commented_forced(&els.comments, expression_alone(&els.value), &[]).indent(),
    ])
}

fn let_(decls: &[LetDeclaration], body_comments: &[Comment], body: &Expr) -> Block {
    let mut blocks = Vec::new();
    for (i, decl) in decls.iter().enumerate() {
        // Upstream's spacer: a blank line after each definition.
        if i > 0
            && matches!(
                &decls[i - 1],
                LetDeclaration::Common(d) if matches!(**d, CommonDeclaration::Definition(..))
            )
        {
            blocks.push(Block::blank_line());
        }
        blocks.push(match decl {
            LetDeclaration::Common(decl) => common(decl),
            LetDeclaration::Comment(c) => comment(c),
        });
    }
    text("let")
        .and_then(blocks.into_iter().map(Block::indent))
        .and_then([
            text("in"),
            Block::stack1(
                body_comments
                    .iter()
                    .map(comment)
                    .chain([expression_alone(body)]),
            ),
        ])
}

fn case(subject: &C2<Expr>, multiline: bool, branches: &[CaseBranch]) -> Block {
    let opening = match commented_expression(subject) {
        Block::SingleLine(subject) if !multiline => Block::line(Line::row([
            Line::text("case"),
            Line::space(),
            subject,
            Line::space(),
            Line::text("of"),
        ])),
        subject => Block::stack1([text("case"), subject.indent(), text("of")]),
    };
    let mut clauses = Vec::new();
    for (i, branch) in branches.iter().enumerate() {
        if i > 0 {
            clauses.push(Block::blank_line());
        }
        clauses.push(case_branch(branch).indent());
    }
    opening.and_then(clauses)
}

/// Upstream's `clause` in `formatExpression`'s `Case`.
fn case_branch(branch: &CaseBranch) -> Block {
    let workaround = |block| negative_case_pattern_workaround(&branch.pattern, block);
    let pattern = workaround(syntax_parens(
        SyntaxSeparated,
        format_pattern(&branch.pattern),
    ));
    let stacked = workaround(Block::stack1(
        branch
            .before_pattern
            .iter()
            .map(comment)
            .chain([syntax_parens(
                SyntaxSeparated,
                format_pattern(&branch.pattern),
            )])
            .chain(branch.before_arrow.iter().map(comment)),
    ));
    let body = Block::stack1(
        branch
            .after_arrow
            .iter()
            .map(comment)
            .chain([expression_alone(&branch.body)]),
    );
    let arrow = |pattern| Block::line(Line::row([pattern, Line::space(), Line::text("->")]));
    match (stacked, pattern) {
        (Block::SingleLine(pattern), _) => Block::stack1([arrow(pattern), body.indent()]),
        (_, Block::SingleLine(pattern)) if branch.before_arrow.is_empty() => Block::stack1(
            branch
                .before_pattern
                .iter()
                .map(comment)
                .chain([arrow(pattern), body.indent()]),
        ),
        (stacked, _) => Block::stack1([stacked, text("->").indent(), body.indent().indent()]),
    }
}

/// Upstream parenthesizes a negative literal a case branch matches.
fn negative_case_pattern_workaround(pattern: &Pattern, block: Block) -> Block {
    match pattern {
        Pattern::Literal(LiteralValue::Int(i, _)) if *i < 0 => parens(block),
        Pattern::Literal(LiteralValue::Float(f, _)) if *f < 0.0 => parens(block),
        _ => block,
    }
}

#[cfg(test)]
mod tests;
