//! Types, after upstream's `Parse/Type.hs`.

use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::{
    VALID, commas, eol_after, lower_name, pair, parens_or_bare, preceded, sectioned, separated,
    start, surround, upper_ref, with_eol,
};
use crate::tree::{Around, AroundEol, Type, TypeConstructor, c};

pub(super) fn ty(cx: &mut Cursor, node: &ast::Type) -> Type {
    nomi_base::grow_large(|| ty_unguarded(cx, node))
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
            let item = surround(cx, t.syntax(), t.ty().into_iter(), &close, |cx, t| {
                with_eol(cx, t, ty)
            })
            .pop()
            .expect(VALID);
            parens_or_bare(eol_after(item), Type::Parens)
        }
        ast::Type::TupleType(t) => {
            let close = t.r_paren_token().expect(VALID);
            let items = surround(cx, t.syntax(), t.types(), &close, |cx, t| {
                with_eol(cx, t, ty)
            })
            .into_iter()
            .map(|item| {
                let Around { before, after } = item.comments;
                let eol = item.value.comments;
                c(AroundEol { before, after, eol }, item.value.value)
            })
            .collect();
            Type::Tuple(items, cx.force_multiline(t.syntax()))
        }
        ast::Type::RecordType(t) => record(cx, t),
    }
}

/// A constructor and its arguments, each with the comments before it;
/// upstream's `app`.
fn construction(
    cx: &mut Cursor,
    node: &ast::Type,
    ctor: TypeConstructor,
    args: impl Iterator<Item = ast::Type>,
) -> Type {
    let args = preceded(cx, args, ty);
    Type::Construction {
        ctor,
        args,
        force_multiline: cx.force_multiline(node.syntax()),
    }
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
        force_multiline: cx.force_multiline(node.syntax()),
    }
}

/// `{ base | field : type, … }`; upstream's `record`.
fn record(cx: &mut Cursor, node: &ast::RecordType) -> Type {
    let close = node.r_brace_token().expect(VALID);
    let base = node.record_extension().map(|extension| {
        let name = extension.lower_ident_token().expect(VALID);
        let before = cx.take(start(&name));
        let after = cx.take(start(&extension.pipe_token().expect(VALID)));
        c(Around { before, after }, lower_name(&name))
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
        force_multiline: cx.force_multiline(node.syntax()),
    }
}

#[cfg(test)]
mod tests;
