//! Types, after upstream's `Parse/Type.hs`.

use nomi_syntax::SyntaxKind;
use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::{VALID, c, commas, lower_name, range, separated, start, surround, token, upper_ref};
use crate::tree::{C0Eol, Comment, ForceMultiline, Lower, Pair, Sequence, Type, TypeConstructor};

pub(super) fn ty(cx: &mut Cursor, node: &ast::Type) -> Type {
    match node {
        ast::Type::FunctionType(f) => function(cx, f),
        ast::Type::NamedType(t) => {
            let name = upper_ref(t.syntax(), t.qualifier());
            construction(cx, node, TypeConstructor::Named(name), t.types())
        }
        ast::Type::TupleConstructorType(t) => {
            let size = commas(t.syntax()).len() + 1;
            construction(cx, node, TypeConstructor::Tuple(size), t.types())
        }
        ast::Type::TypeVar(t) => Type::Variable(lower_name(&t.lower_ident_token().expect(VALID))),
        ast::Type::UnitType(t) => Type::Unit(cx.take(start(&t.r_paren_token().expect(VALID)))),
        ast::Type::ParenType(t) => {
            let close = t.r_paren_token().expect(VALID);
            let item = surround(cx, t.syntax(), t.ty().into_iter(), &close, with_eol)
                .pop()
                .expect(VALID);
            let ((pre, post), inner) = (item.comments, item.value);
            // Without comments, upstream drops the parentheses.
            match inner.comments {
                None if pre.is_empty() && post.is_empty() => inner.value,
                eol => {
                    let mut comments: Vec<Comment> = eol.map(Comment::Line).into_iter().collect();
                    comments.extend(post);
                    Type::Parens(Box::new(c((pre, comments), inner.value)))
                }
            }
        }
        ast::Type::TupleType(t) => {
            let close = t.r_paren_token().expect(VALID);
            let items = surround(cx, t.syntax(), t.types(), &close, with_eol)
                .into_iter()
                .map(|item| {
                    let ((pre, post), eol) = (item.comments, item.value.comments);
                    c((pre, post, eol), item.value.value)
                })
                .collect();
            Type::Tuple(items, multiline(cx, node))
        }
        ast::Type::RecordType(t) => record(cx, t),
    }
}

/// A type with the `--` comment ending its line, unless it spans lines;
/// upstream's `withEol`.
fn with_eol(cx: &mut Cursor, node: &ast::Type) -> C0Eol<Type> {
    let value = ty(cx, node);
    c(cx.eol(range(node.syntax())), value)
}

fn multiline(cx: &Cursor, node: &ast::Type) -> ForceMultiline {
    ForceMultiline(cx.multiline(range(node.syntax())))
}

/// A constructor and its arguments, each with the comments before it;
/// upstream's `app`.
fn construction(
    cx: &mut Cursor,
    node: &ast::Type,
    ctor: TypeConstructor,
    args: impl Iterator<Item = ast::Type>,
) -> Type {
    let args = args
        .map(|arg| {
            let comments = cx.take(range(arg.syntax()).start());
            c(comments, ty(cx, &arg))
        })
        .collect();
    Type::Construction(ctor, args, multiline(cx, node))
}

/// `a -> b -> c`, which the syntax tree nests to the right.
fn function(cx: &mut Cursor, node: &ast::FunctionType) -> Type {
    let mut terms = Vec::new();
    let mut arrows = Vec::new();
    let mut link = node.clone();
    loop {
        terms.push(link.param().expect(VALID));
        arrows.push(link.arrow_token().expect(VALID));
        match link.result().expect(VALID) {
            ast::Type::FunctionType(next) => link = next,
            last => {
                terms.push(last);
                break;
            }
        }
    }
    let (first, rest) = separated(cx, terms, arrows, ty);
    Type::Function {
        first: Box::new(first),
        rest,
        force_multiline: ForceMultiline(cx.multiline(range(node.syntax()))),
    }
}

/// `{ base | field : type, … }`; upstream's `record`, its fields a
/// `sectionedGroup`.
fn record(cx: &mut Cursor, node: &ast::RecordType) -> Type {
    let close = node.r_brace_token().expect(VALID);
    let base = node.record_extension().map(|extension| {
        let name = extension.lower_ident_token().expect(VALID);
        let pre = cx.take(start(&name));
        let post = cx.take(start(&extension.pipe_token().expect(VALID)));
        c((pre, post), lower_name(&name))
    });
    let commas = commas(node.syntax());
    let mut leading = Vec::new();
    let mut fields = Vec::new();
    for (i, field) in node.field_types().enumerate() {
        let pre = cx.take(range(field.syntax()).start());
        let pair = pair(cx, &field);
        let eol = cx.eol(range(field.syntax()));
        let next = commas.get(i).copied().unwrap_or_else(|| start(&close));
        let before_next = cx.take(next);
        fields.push(c(
            (std::mem::replace(&mut leading, before_next), pre, eol),
            pair,
        ));
    }
    // With no fields, upstream reads the comments inside as trailing.
    let trailing = if fields.is_empty() {
        cx.take(start(&close))
    } else {
        leading
    };
    Type::Record {
        base,
        fields: Sequence(fields),
        trailing,
        force_multiline: ForceMultiline(cx.multiline(range(node.syntax()))),
    }
}

/// `name : type`, or the legacy `name = type`; upstream's `pair`.
fn pair(cx: &mut Cursor, field: &ast::FieldType) -> Pair<Lower, Type> {
    let name = field.lower_ident_token().expect(VALID);
    let separator = token(field.syntax(), SyntaxKind::Colon)
        .or_else(|| token(field.syntax(), SyntaxKind::Eq))
        .expect(VALID);
    let key = c(cx.take(start(&separator)), lower_name(&name));
    let value = field.ty().expect(VALID);
    let value = c(cx.take(range(value.syntax()).start()), ty(cx, &value));
    Pair {
        key,
        value,
        force_multiline: ForceMultiline(cx.multiline(range(field.syntax()))),
    }
}

#[cfg(test)]
mod tests;
