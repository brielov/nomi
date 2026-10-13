//! Patterns, after upstream's `Parse/Pattern.hs`.

use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::literal::literal;
use super::{
    VALID, bool_literal, lower_name, parens_or_bare, preceded, separated, start, surround,
    upper_ref,
};
use crate::tree::{LiteralValue, Pattern, c};

pub(super) fn pattern(cx: &mut Cursor, node: &ast::Pattern) -> Pattern {
    nomi_base::grow_large(|| pattern_unguarded(cx, node))
}

fn pattern_unguarded(cx: &mut Cursor, node: &ast::Pattern) -> Pattern {
    match node {
        ast::Pattern::WildcardPattern(_) => Pattern::Anything,
        ast::Pattern::VarPattern(p) => {
            Pattern::Var(lower_name(&p.lower_ident_token().expect(VALID)))
        }
        ast::Pattern::LiteralPattern(p) => Pattern::Literal(literal(p.syntax())),
        ast::Pattern::ConstructorPattern(p) => constructor(cx, p),
        ast::Pattern::UnitPattern(p) => {
            Pattern::Unit(cx.take(start(&p.r_paren_token().expect(VALID))))
        }
        ast::Pattern::ParenPattern(p) => {
            let close = p.r_paren_token().expect(VALID);
            let item = surround(cx, p.syntax(), p.pattern().into_iter(), &close, pattern)
                .pop()
                .expect(VALID);
            parens_or_bare(item, Pattern::Parens)
        }
        ast::Pattern::TuplePattern(p) => {
            let close = p.r_paren_token().expect(VALID);
            Pattern::Tuple(surround(cx, p.syntax(), p.patterns(), &close, pattern))
        }
        ast::Pattern::ListPattern(p) => {
            let close = p.r_bracket_token().expect(VALID);
            let items = surround(cx, p.syntax(), p.patterns(), &close, pattern);
            if items.is_empty() {
                Pattern::EmptyList(cx.take(start(&close)))
            } else {
                Pattern::List(items)
            }
        }
        ast::Pattern::RecordPattern(p) => {
            let close = p.r_brace_token().expect(VALID);
            let fields = surround(cx, p.syntax(), p.field_patterns(), &close, |_, field| {
                lower_name(&field.lower_ident_token().expect(VALID))
            });
            if fields.is_empty() {
                Pattern::EmptyRecord(cx.take(start(&close)))
            } else {
                Pattern::Record(fields)
            }
        }
        ast::Pattern::ConsPattern(p) => cons(cx, p),
        ast::Pattern::AliasPattern(p) => {
            let inner = pattern(cx, &p.pattern().expect(VALID));
            let name = p.lower_ident_token().expect(VALID);
            let before_as = cx.take(start(&p.as_token().expect(VALID)));
            let after_as = cx.take(start(&name));
            Pattern::Alias(
                Box::new(c(before_as, inner)),
                c(after_as, lower_name(&name)),
            )
        }
    }
}

/// A constructor and its arguments, each with the comments before it;
/// an unqualified `True` or `False` is a literal.
fn constructor(cx: &mut Cursor, node: &ast::ConstructorPattern) -> Pattern {
    let name = upper_ref(node.syntax(), node.qualifier());
    let mut args = node.patterns().peekable();
    if args.peek().is_none()
        && let Some(value) = bool_literal(&name.namespace, &name.name)
    {
        return Pattern::Literal(LiteralValue::Bool(value));
    }
    Pattern::Data(name, preceded(cx, args, pattern))
}

/// `a :: b :: c`, which the syntax tree nests to the right.
fn cons(cx: &mut Cursor, node: &ast::ConsPattern) -> Pattern {
    let mut terms = Vec::new();
    let mut separators = Vec::new();
    let mut link = node.clone();
    loop {
        terms.push(link.head().expect(VALID));
        separators.push(link.colon_colon_token().expect(VALID));
        match link.tail().expect(VALID) {
            ast::Pattern::ConsPattern(next) => link = next,
            last => {
                terms.push(last);
                break;
            }
        }
    }
    let (first, rest) = separated(cx, terms, &separators, pattern);
    Pattern::Cons {
        first: Box::new(first),
        rest,
    }
}

#[cfg(test)]
mod tests;
