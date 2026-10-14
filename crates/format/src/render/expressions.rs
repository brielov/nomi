//! Expressions; upstream's `formatExpression` and the helpers only it
//! uses.

use crate::tree::{
    ApplicationMultiline, BinopsClause, C1, C2, CaseBranch, Comment, CommonDeclaration, Expr,
    ForceMultiline, IfClause, LetDeclaration, LiteralValue, Lower, Multiline, Pattern, Ref, Upper,
};

use super::SyntaxContext::{self, AmbiguousEnd, InfixSeparated, SpaceSeparated, SyntaxSeparated};
use super::block::{Block, Line};
use super::declarations::common;
use super::patterns::{pattern_alone, pattern_argument};
use super::structure::{Delimiters, application, group, interleave, intersperse};
use super::{
    BinaryOp, binary, comment, commented, commented_forced, literal::literal, name, operator, pair,
    parens, pre_commented, push_name, qualified, record_like, sequence, syntax_parens, text,
    tuple_function, unit,
};

/// `expr` where nothing around it calls for parentheses.
pub(super) fn expression_alone(expr: &Expr) -> Block {
    syntax_parens(SyntaxSeparated, format_expression(expr))
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
        Ref::Var(namespace, Lower(last)) | Ref::Tag(namespace, Upper(last)) => {
            qualified_name(namespace, last)
        }
        Ref::Op(op) => operator(op),
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
    nomi_base::grow_large(|| format_expression_unguarded(expr))
}

#[expect(
    clippy::too_many_lines,
    reason = "upstream's `formatExpression`, one short arm per expression"
)]
fn format_expression_unguarded(expr: &Expr) -> (SyntaxContext, Block) {
    match expr {
        Expr::Literal(value) => (SyntaxSeparated, literal(value)),
        Expr::Var(v) => (SyntaxSeparated, Block::line(var(v))),
        Expr::Range { low, high } => (SpaceSeparated, range(low, high)),
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
        Expr::Binops {
            first,
            clauses,
            multiline,
        } => (InfixSeparated, binops(first, clauses, *multiline)),
        Expr::Lambda {
            args,
            before_body,
            body,
            multiline,
        } => (AmbiguousEnd, lambda(args, before_body, body, *multiline)),
        Expr::Negate(e) => {
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
        Expr::App { function, args, .. } if args.is_empty() => format_expression(function),
        Expr::App {
            function,
            args,
            multiline,
        } => (
            SpaceSeparated,
            application(
                *multiline,
                syntax_parens(InfixSeparated, format_expression(function)),
                args.iter()
                    .map(|a| pre_commented_expression(SpaceSeparated, &a.comments, &a.value))
                    .collect(),
            ),
        ),
        Expr::If {
            first,
            else_ifs,
            else_body,
        } => (AmbiguousEnd, if_(first, else_ifs, else_body)),
        Expr::Let {
            declarations,
            before_body,
            body,
        } => (AmbiguousEnd, let_(declarations, before_body, body)),
        Expr::Case {
            subject,
            multiline,
            branches,
        } => (AmbiguousEnd, case(subject, *multiline, branches)),
        Expr::Tuple(exprs, multiline) => (
            SyntaxSeparated,
            group(
                Delimiters::PADDED_PARENS,
                *multiline,
                exprs.iter().map(commented_expression).collect(),
            ),
        ),
        Expr::TupleFunction(n) => (SyntaxSeparated, Block::line(tuple_function(*n))),
        Expr::Access(e, field) => (
            SyntaxSeparated,
            syntax_parens(SpaceSeparated, format_expression(e))
                .add_suffix(Line::row([Line::text("."), name(&field.0)])),
        ),
        Expr::AccessFunction(field) => {
            let mut accessor = String::from(".");
            push_name(&mut accessor, &field.0);
            (SyntaxSeparated, text(accessor))
        }
        Expr::Record {
            base,
            fields,
            trailing,
            force_multiline,
        } => (
            SyntaxSeparated,
            record_like(base.as_ref(), fields, trailing, *force_multiline, |field| {
                pair("=", field, expression_alone)
            }),
        ),
        Expr::Parens(inner) if inner.comments.is_empty() => format_expression(&inner.value),
        Expr::Parens(inner) => (SyntaxSeparated, parens(commented_expression(inner))),
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
        Expr::Negate(..) => true,
        Expr::Literal(LiteralValue::Int(n, _)) => *n < 0,
        Expr::Literal(LiteralValue::Float(f, _)) => f.is_sign_negative(),
        Expr::Parens(inner) if inner.comments.is_empty() => starts_with_minus(&inner.value),
        Expr::Access(record, _) => starts_with_minus(record),
        Expr::Literal(_)
        | Expr::Unit(_)
        | Expr::Var(_)
        | Expr::App { .. }
        | Expr::Binops { .. }
        | Expr::Parens(_)
        | Expr::List { .. }
        | Expr::Range { .. }
        | Expr::Tuple(..)
        | Expr::TupleFunction(_)
        | Expr::Record { .. }
        | Expr::AccessFunction(_)
        | Expr::Lambda { .. }
        | Expr::If { .. }
        | Expr::Let { .. }
        | Expr::Case { .. }
        | Expr::GlShader(_) => false,
    }
}

/// Upstream's `formatCommentedExpression`: parentheses with comments
/// inside give theirs up to the slot around them.
fn commented_expression(expr: &C2<Expr>) -> Block {
    let around = &expr.comments;
    match &expr.value {
        Expr::Parens(inner) => commented(
            around.before.iter().chain(&inner.comments.before),
            expression_alone(&inner.value),
            inner.comments.after.iter().chain(&around.after),
        ),
        value => commented(&around.before, expression_alone(value), &around.after),
    }
}

/// Upstream's `formatPreCommentedExpression`, of `expr` after the
/// comments `pre`.
fn pre_commented_expression(context: SyntaxContext, pre: &[Comment], expr: &Expr) -> Block {
    match expr {
        Expr::Parens(inner) if inner.comments.after.is_empty() => pre_commented(
            pre.iter().chain(&inner.comments.before),
            syntax_parens(context, format_expression(&inner.value)),
        ),
        value => pre_commented(pre, syntax_parens(context, format_expression(value))),
    }
}

/// `[a..b]` as Elm 0.19 spells it, `List.range a b`; upstream's
/// `formatRange_0_18`, which formats that call.
fn range(low: &C2<Expr>, high: &C2<Expr>) -> Block {
    // Upstream parenthesizes both operands when either has comments after.
    let parenthesized = !(low.comments.after.is_empty() && high.comments.after.is_empty());
    let argument = |bound: &C2<Expr>| match &bound.comments.after[..] {
        _ if !parenthesized => {
            pre_commented_expression(SpaceSeparated, &bound.comments.before, &bound.value)
        }
        [] => pre_commented(
            &bound.comments.before,
            syntax_parens(SpaceSeparated, format_expression(&bound.value)),
        ),
        _ => parens(commented_expression(bound)),
    };
    application(
        ApplicationMultiline::JoinFirst(Multiline::JoinAll),
        text("List.range"),
        vec![argument(low), argument(high)],
    )
}

/// Upstream's `formatBinops`.
fn binops(first: &Expr, clauses: &[BinopsClause], multiline: ForceMultiline) -> Block {
    let last = clauses.len().saturating_sub(1);
    let ops = clauses
        .iter()
        .enumerate()
        .map(|(i, clause)| {
            let left_pipe = matches!(&clause.op, Ref::Op(op) if op.is_left_pipe());
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
        syntax_parens(InfixSeparated, format_expression(first)),
        ops,
    )
}

/// `\patterns -> body`.
fn lambda(
    args: &[C1<Pattern>],
    before_body: &[Comment],
    body: &Expr,
    multiline: ForceMultiline,
) -> Block {
    let args = args
        .iter()
        .map(|p| pre_commented(&p.comments, pattern_argument(&p.value)))
        .collect();
    let body = expression_alone(body);
    let indented_body =
        |body| Block::stack1(before_body.iter().map(comment).chain([body])).indent();
    match Block::all_singles(args) {
        Ok(args) => {
            let mut head = Line::row([
                Line::text("\\"),
                intersperse(args, Line::space),
                Line::space(),
                Line::text("->"),
            ]);
            match body {
                Block::SingleLine(body) if !multiline.0 && before_body.is_empty() => {
                    head.push(Line::space());
                    head.push(body);
                    Block::line(head)
                }
                body => Block::stack1([Block::line(head), indented_body(body)]),
            }
        }
        Err(args) => Block::stack1([
            Block::stack1(args).prefix(&Line::text("\\")),
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
    commented_forced(
        &body.comments.before,
        expression_alone(&body.value),
        &body.comments.after,
    )
    .indent()
}

fn if_(first: &IfClause, else_ifs: &[C1<IfClause>], else_body: &C1<Expr>) -> Block {
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
        commented_forced(&else_body.comments, expression_alone(&else_body.value), &[]).indent(),
    ])
}

fn let_(declarations: &[LetDeclaration], before_body: &[Comment], body: &Expr) -> Block {
    let mut blocks = Vec::new();
    for (i, declaration) in declarations.iter().enumerate() {
        // Upstream's spacer: a blank line after each definition.
        if i > 0
            && matches!(
                &declarations[i - 1],
                LetDeclaration::Common(d) if matches!(**d, CommonDeclaration::Definition { .. })
            )
        {
            blocks.push(Block::blank_line());
        }
        blocks.push(match declaration {
            LetDeclaration::Common(declaration) => common(declaration),
            LetDeclaration::Comment(c) => comment(c),
        });
    }
    text("let")
        .and_then(blocks.into_iter().map(Block::indent))
        .and_then([
            text("in"),
            Block::stack1(
                before_body
                    .iter()
                    .map(comment)
                    .chain([expression_alone(body)]),
            ),
        ])
}

fn case(subject: &C2<Expr>, multiline: ForceMultiline, branches: &[CaseBranch]) -> Block {
    let opening = match commented_expression(subject) {
        Block::SingleLine(subject) if !multiline.0 => Block::line(Line::row([
            Line::text("case"),
            Line::space(),
            subject,
            Line::space(),
            Line::text("of"),
        ])),
        subject => Block::stack1([text("case"), subject.indent(), text("of")]),
    };
    opening.and_then(interleave(
        branches.iter().map(|branch| case_branch(branch).indent()),
        Block::blank_line,
    ))
}

/// Upstream's `clause` in `formatExpression`'s `Case`.
fn case_branch(branch: &CaseBranch) -> Block {
    let workaround = |block| negative_case_pattern_workaround(&branch.pattern, block);
    let alone = pattern_alone(&branch.pattern);
    let before_pattern = branch.before_pattern.iter().map(comment);
    let body = Block::stack1(
        branch
            .after_arrow
            .iter()
            .map(comment)
            .chain([expression_alone(&branch.body)]),
    );
    if alone.is_line() && branch.before_arrow.is_empty() {
        let pattern = (workaround(alone).into_line()).expect("parentheses keep a line one line");
        let arrow = Block::line(Line::row([pattern, Line::space(), Line::text("->")]));
        return Block::stack1(before_pattern.chain([arrow, body.indent()]));
    }
    let stacked = workaround(Block::stack1(
        before_pattern
            .chain([alone])
            .chain(branch.before_arrow.iter().map(comment)),
    ));
    Block::stack1([stacked, text("->").indent(), body.indent().indent()])
}

/// Upstream parenthesizes a negative literal a case branch matches.
fn negative_case_pattern_workaround(pattern: &Pattern, block: Block) -> Block {
    match pattern {
        Pattern::Literal(LiteralValue::Int(i, _)) if *i < 0 => parens(block),
        Pattern::Literal(LiteralValue::Float(f, _)) if *f < 0.0 => parens(block),
        Pattern::Literal(_)
        | Pattern::Anything
        | Pattern::Unit(_)
        | Pattern::Var(_)
        | Pattern::Op(_)
        | Pattern::Data(..)
        | Pattern::Parens(_)
        | Pattern::Tuple(_)
        | Pattern::EmptyList(_)
        | Pattern::List(_)
        | Pattern::Cons { .. }
        | Pattern::EmptyRecord(_)
        | Pattern::Record(_)
        | Pattern::Alias(..) => block,
    }
}

#[cfg(test)]
mod tests;
