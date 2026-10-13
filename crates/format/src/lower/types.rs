//! Types, after upstream's `Parse/Type.hs`.

use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::{VALID, c, commas, lower_name, pair, sectioned, separated, start, surround, upper_ref};
use crate::tree::{C0Eol, Comment, ForceMultiline, Type, TypeConstructor};

pub(super) fn ty(cx: &mut Cursor, node: &ast::Type) -> Type {
    crate::guarded(|| ty_unguarded(cx, node))
}

fn ty_unguarded(cx: &mut Cursor, node: &ast::Type) -> Type {
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
    c(cx.eol(cx.range(node.syntax())), value)
}

fn multiline(cx: &Cursor, node: &ast::Type) -> ForceMultiline {
    ForceMultiline(cx.multiline(cx.range(node.syntax())))
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
            let comments = cx.take(cx.range(arg.syntax()).start());
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
    let (first, rest) = separated(cx, terms, &arrows, ty);
    Type::Function {
        first: Box::new(first),
        rest,
        force_multiline: ForceMultiline(cx.multiline(cx.range(node.syntax()))),
    }
}

/// `{ base | field : type, … }`; upstream's `record`.
fn record(cx: &mut Cursor, node: &ast::RecordType) -> Type {
    let close = node.r_brace_token().expect(VALID);
    let base = node.record_extension().map(|extension| {
        let name = extension.lower_ident_token().expect(VALID);
        let pre = cx.take(start(&name));
        let post = cx.take(start(&extension.pipe_token().expect(VALID)));
        c((pre, post), lower_name(&name))
    });
    let (fields, trailing) = sectioned(
        cx,
        node.syntax(),
        node.field_types(),
        &close,
        |cx, field| pair(cx, field.syntax(), &field.ty().expect(VALID), ty),
    );
    Type::Record {
        base,
        fields,
        trailing,
        force_multiline: ForceMultiline(cx.multiline(cx.range(node.syntax()))),
    }
}

#[cfg(test)]
mod tests;
