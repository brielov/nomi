//! Top-level declarations, after upstream's `Parse/Declaration.hs` and
//! `topLevel` in `Parse/Module.hs`.

use nomi_base::TextSize;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxToken};

use super::cursor::Cursor;
use super::definitions::{annotation, definition, destructure};
use super::types::ty;
use super::{VALID, c, lower_name, range, start, symbol, token, upper_name};
use crate::tree::{
    Assoc, C1, C2, Declaration, Lower, NameWithArgs, OpenCommentedList, TopLevelStructure, Upper,
};

type Body = Vec<TopLevelStructure<Declaration>>;

/// `decl` onto `body`: its doc comment, its parts, and the comments
/// before each, which upstream reads as comments between declarations.
pub(super) fn declaration(cx: &mut Cursor, body: &mut Body, decl: &ast::Decl) {
    let doc = match decl {
        ast::Decl::ValueDecl(d) => d.doc_comment_token(),
        ast::Decl::DestructureDecl(d) => d.doc_comment_token(),
        ast::Decl::TypeAliasDecl(d) => d.doc_comment_token(),
        ast::Decl::CustomTypeDecl(d) => d.doc_comment_token(),
        ast::Decl::PortDecl(d) => d.doc_comment_token(),
        ast::Decl::DocDecl(d) => d.doc_comment_token(),
        ast::Decl::InfixDecl(_) => None,
    };
    if let Some(doc) = doc {
        comments_before(cx, body, start(&doc));
        body.push(TopLevelStructure::DocComment(super::docs::docs(&doc)));
    }
    let entry = |d| TopLevelStructure::Entry(Declaration::Common(d));
    match decl {
        ast::Decl::ValueDecl(d) => {
            if let Some(signature) = d.signature() {
                comments_before(cx, body, range(signature.syntax()).start());
                body.push(entry(annotation(cx, &signature)));
            }
            // Upstream reads an annotation on its own, so either may be
            // missing.
            if let Some(value) = d.definition() {
                comments_before(cx, body, range(value.syntax()).start());
                body.push(entry(definition(cx, &value)));
            }
        }
        ast::Decl::DestructureDecl(d) => {
            let target = d.pattern().expect(VALID);
            comments_before(cx, body, range(target.syntax()).start());
            let (eq, value) = (d.eq_token().expect(VALID), d.expr().expect(VALID));
            body.push(entry(destructure(cx, &target, &eq, &value)));
        }
        ast::Decl::TypeAliasDecl(d) => {
            comments_before(cx, body, start(&d.type_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(alias(cx, d)));
        }
        ast::Decl::CustomTypeDecl(d) => {
            comments_before(cx, body, start(&d.type_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(custom_type(cx, d)));
        }
        ast::Decl::PortDecl(d) => {
            comments_before(cx, body, start(&d.port_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(port(cx, d)));
        }
        ast::Decl::InfixDecl(d) => {
            comments_before(cx, body, start(&d.infix_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(infix(cx, d)));
        }
        ast::Decl::DocDecl(_) => {}
    }
}

/// The comments before `end` onto `body`.
fn comments_before(cx: &mut Cursor, body: &mut Body, end: TextSize) {
    let comments = cx.take(end);
    body.extend(comments.into_iter().map(TopLevelStructure::BodyComment));
}

/// `type alias Name args = type`.
fn alias(cx: &mut Cursor, node: &ast::TypeAliasDecl) -> Declaration {
    let after_type = cx.take(start(&node.alias_token().expect(VALID)));
    let name = node.upper_ident_token().expect(VALID);
    let after_alias = cx.take(start(&name));
    let name_with_args = name_with_args(cx, &name, node.type_params());
    let before_eq = cx.take(start(&node.eq_token().expect(VALID)));
    let aliased = node.ty().expect(VALID);
    let after_eq = cx.take(range(aliased.syntax()).start());
    Declaration::TypeAlias(
        after_type,
        c((after_alias, before_eq), name_with_args),
        c(after_eq, ty(cx, &aliased)),
    )
}

/// `type Name args = A | B`: each variant with the `--` comment that may
/// end its line, and the comments around the `|` after it; upstream's
/// `pipeSep1` and `exposedToOpen`.
fn custom_type(cx: &mut Cursor, node: &ast::CustomTypeDecl) -> Declaration {
    let name = node.upper_ident_token().expect(VALID);
    let after_type = cx.take(start(&name));
    let name_with_args = name_with_args(cx, &name, node.type_params());
    let before_eq = cx.take(start(&node.eq_token().expect(VALID)));
    let mut pipes = node
        .syntax()
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::Pipe);
    let mut variants = node.variants().peekable();
    let mut items = Vec::new();
    let last = loop {
        // After `=` or `|`, which the previous slot stopped before.
        let variant = variants.next().expect(VALID);
        let pre = cx.take(range(variant.syntax()).start());
        let name = upper_name(&variant.upper_ident_token().expect(VALID));
        let args = variant
            .types()
            .map(|arg| c(cx.take(range(arg.syntax()).start()), ty(cx, &arg)))
            .collect();
        let value = NameWithArgs { name, args };
        let eol = cx.eol(range(variant.syntax()));
        if variants.peek().is_none() {
            break c((pre, eol), value);
        }
        let post = cx.take(start(&pipes.next().expect(VALID)));
        items.push(c((pre, post, eol), value));
    };
    Declaration::Datatype {
        name_with_args: c((after_type, before_eq), name_with_args),
        variants: OpenCommentedList { items, last },
    }
}

/// A type's name and parameters, each with the comments before it.
fn name_with_args(
    cx: &mut Cursor,
    name: &SyntaxToken,
    params: impl Iterator<Item = ast::TypeParam>,
) -> NameWithArgs<Upper, Lower> {
    let args = params
        .map(|param| {
            let param = param.lower_ident_token().expect(VALID);
            c(cx.take(start(&param)), lower_name(&param))
        })
        .collect();
    NameWithArgs {
        name: upper_name(name),
        args,
    }
}

/// `port name : type`.
fn port(cx: &mut Cursor, node: &ast::PortDecl) -> Declaration {
    let name = node.lower_ident_token().expect(VALID);
    let before_name = cx.take(start(&name));
    let after_name = cx.take(start(&node.colon_token().expect(VALID)));
    let annotation = node.ty().expect(VALID);
    let before_type = cx.take(range(annotation.syntax()).start());
    Declaration::PortAnnotation(
        c((before_name, after_name), lower_name(&name)),
        before_type,
        ty(cx, &annotation),
    )
}

/// `infix left 6 (+) = add`.
fn infix(cx: &mut Cursor, node: &ast::InfixDecl) -> Declaration {
    let syntax = node.syntax();
    let (assoc, keyword) = [
        (Assoc::Left, SyntaxKind::LeftKw),
        (Assoc::Right, SyntaxKind::RightKw),
        (Assoc::Non, SyntaxKind::NonKw),
    ]
    .into_iter()
    .find_map(|(assoc, kind)| token(syntax, kind).map(|t| (assoc, t)))
    .expect(VALID);
    let assoc: C1<Assoc> = c(cx.take(start(&keyword)), assoc);
    let int = node.int_token().expect(VALID);
    let precedence = c(cx.take(start(&int)), int.text().parse().expect(VALID));
    let before_op = cx.take(start(&node.l_paren_token().expect(VALID)));
    let after_op = cx.take(start(&node.eq_token().expect(VALID)));
    let op: C2<_> = c((before_op, after_op), symbol(&node.op().expect(VALID)));
    let function = node.lower_ident_token().expect(VALID);
    let function = c(cx.take(start(&function)), lower_name(&function));
    Declaration::Fixity(assoc, precedence, op, function)
}

#[cfg(test)]
mod tests;
