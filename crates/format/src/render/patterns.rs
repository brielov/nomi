//! Patterns; upstream's `formatPattern`.

use crate::block::{Block, Line};
use crate::structure::{application, group};
use crate::tree::{ApplicationMultiline, Multiline, Pattern};

use super::SyntaxContext::{SpaceSeparated, SyntaxSeparated};
use super::{
    BinaryOp, SyntaxContext, binary, c2, c2_eol, eol_commented, literal::literal, name, parens,
    pre_commented, syntax_parens, tail_commented, unit, upper_ref,
};

/// `pattern` as an argument or an operand: in parentheses unless it is
/// one token or delimited.
pub(super) fn pattern_argument(pattern: &Pattern) -> Block {
    syntax_parens(SpaceSeparated, format_pattern(pattern))
}

/// `pattern` where nothing around it calls for parentheses.
pub(super) fn pattern_alone(pattern: &Pattern) -> Block {
    syntax_parens(SyntaxSeparated, format_pattern(pattern))
}

fn text(text: impl Into<String>) -> Block {
    Block::line(Line::text(text))
}

/// Upstream's `formatPattern`.
pub(super) fn format_pattern(pattern: &Pattern) -> (SyntaxContext, Block) {
    crate::guarded(|| format_pattern_unguarded(pattern))
}

fn format_pattern_unguarded(pattern: &Pattern) -> (SyntaxContext, Block) {
    match pattern {
        Pattern::Anything => (SyntaxSeparated, text("_")),
        Pattern::Unit(comments) => (SyntaxSeparated, unit('(', ')', comments)),
        Pattern::Literal(value) => (SyntaxSeparated, literal(value)),
        Pattern::Var(var) => (SyntaxSeparated, Block::line(name(&var.0))),
        Pattern::Op(op) => (SyntaxSeparated, text(format!("({})", op.0))),
        Pattern::Cons { first, rest } => {
            let left = eol_commented(first.comments.as_deref(), pattern_argument(&first.value));
            let ops = rest
                .0
                .iter()
                .map(|item| {
                    let (pre_op, post_op, eol) = &item.comments;
                    BinaryOp {
                        left_pipe: false,
                        comments: pre_op,
                        op: text("::"),
                        next: c2_eol(post_op, &[], eol.as_deref(), pattern_argument(&item.value)),
                    }
                })
                .collect();
            (SpaceSeparated, binary(false, left, ops))
        }
        Pattern::Data(ctor, args) if args.is_empty() => {
            (SyntaxSeparated, Block::line(upper_ref(ctor)))
        }
        Pattern::Data(ctor, args) => {
            let args = args
                .iter()
                .map(|a| pre_commented(&a.comments, pattern_argument(&a.value)))
                .collect();
            (
                SpaceSeparated,
                application(
                    ApplicationMultiline::JoinFirst(Multiline::JoinAll),
                    Block::line(upper_ref(ctor)),
                    args,
                ),
            )
        }
        Pattern::Parens(inner) => (SyntaxSeparated, parens(c2(inner, pattern_alone))),
        Pattern::Tuple(items) => (SyntaxSeparated, delimited("(", ")", items)),
        Pattern::EmptyList(comments) => (SyntaxSeparated, unit('[', ']', comments)),
        Pattern::List(items) => (SyntaxSeparated, delimited("[", "]", items)),
        Pattern::EmptyRecord(comments) => (SyntaxSeparated, unit('{', '}', comments)),
        Pattern::Record(fields) => {
            let fields = fields
                .iter()
                .map(|f| c2(f, |field| Block::line(name(&field.0))))
                .collect();
            (SyntaxSeparated, group(true, "{", ",", "}", false, fields))
        }
        Pattern::Alias(pattern, alias) => {
            let pattern = tail_commented(pattern_argument(&pattern.value), &pattern.comments);
            let alias = pre_commented(&alias.comments, Block::line(name(&alias.value.0)));
            let block = match (pattern, alias) {
                (Block::SingleLine(pattern), Block::SingleLine(alias)) => Block::line(Line::row([
                    pattern,
                    Line::space(),
                    Line::text("as"),
                    Line::space(),
                    alias,
                ])),
                (pattern, alias) => Block::stack1([pattern, text("as"), alias.indent()]),
            };
            (SpaceSeparated, block)
        }
    }
}

fn delimited(left: &str, right: &str, items: &[crate::tree::C2<Pattern>]) -> Block {
    let items = items.iter().map(|i| c2(i, pattern_alone)).collect();
    group(true, left, ",", right, false, items)
}

#[cfg(test)]
mod tests;
