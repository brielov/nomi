//! Top-level declarations, after upstream's `Parse/Declaration.hs` and
//! `topLevel` in `Parse/Module.hs`.

use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxToken, child_token};

use super::cursor::Cursor;
use super::definitions::{destructure, value_parts};
use super::expressions::expr;
use super::types::ty;
use super::{
    VALID, child_tokens, comments_before, lower_name, preceded, start, symbol, upper_name,
};
use crate::tree::{
    Around, AroundEol, Assoc, BeforeEol, Declaration, Lower, NameWithArgs, OpenCommentedList,
    TopLevelStructure, c,
};

type Body = Vec<TopLevelStructure<Declaration>>;

/// `decl` onto `body`: its doc comment, its parts, and the comments
/// before each, which upstream reads as comments between declarations.
pub(super) fn declaration(cx: &mut Cursor, body: &mut Body, decl: &ast::Decl) {
    if let Some(doc) = child_token(decl.syntax(), SyntaxKind::DocComment) {
        comments_before(cx, body, start(&doc));
        body.push(TopLevelStructure::DocComment(super::docs::docs(&doc)));
    }
    let entry = |d| TopLevelStructure::Entry(Declaration::Common(d));
    match decl {
        ast::Decl::ValueDecl(d) => {
            value_parts(cx, d.signature(), d.definition(), |comments, part| {
                body.extend(comments.into_iter().map(TopLevelStructure::BodyComment));
                body.push(entry(part));
            });
        }
        ast::Decl::DestructureDecl(d) => {
            let target = d.pattern().expect(VALID);
            comments_before(cx, body, cx.range(target.syntax()).start());
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
        ast::Decl::LegacyPortDecl(d) => {
            comments_before(cx, body, start(&d.port_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(legacy_port(cx, d)));
        }
        ast::Decl::InfixDecl(d) => {
            comments_before(cx, body, start(&d.infix_token().expect(VALID)));
            body.push(TopLevelStructure::Entry(infix(cx, d)));
        }
        ast::Decl::LegacyInfixDecl(d) => {
            let (assoc, keyword) = associativity([
                (Assoc::Left, d.infixl_token()),
                (Assoc::Right, d.infixr_token()),
                (Assoc::Non, d.infix_token()),
            ]);
            comments_before(cx, body, start(&keyword));
            body.push(TopLevelStructure::Entry(legacy_infix(cx, d, assoc)));
        }
        ast::Decl::DocDecl(_) => {}
    }
}

/// `type alias Name args = type`.
fn alias(cx: &mut Cursor, node: &ast::TypeAliasDecl) -> Declaration {
    let before_alias = cx.take(start(&node.alias_token().expect(VALID)));
    let name = node.upper_ident_token().expect(VALID);
    let after_alias = cx.take(start(&name));
    let name_with_args = name_with_args(cx, &name, node.type_params());
    let before_eq = cx.take(start(&node.eq_token().expect(VALID)));
    let aliased = node.ty().expect(VALID);
    let after_eq = cx.take(cx.range(aliased.syntax()).start());
    Declaration::TypeAlias {
        before_alias,
        name_with_args: c(Around::new(after_alias, before_eq), name_with_args),
        ty: c(after_eq, ty(cx, &aliased)),
    }
}

/// `type Name args = A | B`: each variant with the `--` comment that may
/// end its line, and the comments around the `|` after it; upstream's
/// `pipeSep1` and `exposedToOpen`.
fn custom_type(cx: &mut Cursor, node: &ast::CustomTypeDecl) -> Declaration {
    let name = node.upper_ident_token().expect(VALID);
    let after_type = cx.take(start(&name));
    let name_with_args = name_with_args(cx, &name, node.type_params());
    let before_eq = cx.take(start(&node.eq_token().expect(VALID)));
    let mut pipes = child_tokens(node.syntax(), SyntaxKind::Pipe);
    let mut variants = node.variants().peekable();
    let mut items = Vec::new();
    let last = loop {
        // After `=` or `|`, which the previous slot stopped before.
        let variant = variants.next().expect(VALID);
        let before = cx.take(cx.range(variant.syntax()).start());
        let name = upper_name(&variant.upper_ident_token().expect(VALID));
        let args = preceded(cx, variant.types(), ty);
        let value = NameWithArgs { name, args };
        let eol = cx.eol(cx.range(variant.syntax()));
        if variants.peek().is_none() {
            break c(BeforeEol { before, eol }, value);
        }
        let after = cx.take(start(&pipes.next().expect(VALID)));
        items.push(c(AroundEol { before, after, eol }, value));
    };
    Declaration::Datatype {
        name_with_args: c(Around::new(after_type, before_eq), name_with_args),
        variants: OpenCommentedList { items, last },
    }
}

/// A type's name and parameters, each with the comments before it.
fn name_with_args(
    cx: &mut Cursor,
    name: &SyntaxToken,
    params: impl Iterator<Item = ast::TypeParam>,
) -> NameWithArgs<Lower> {
    let args = preceded(cx, params, |_, param| {
        lower_name(&param.lower_ident_token().expect(VALID))
    });
    NameWithArgs {
        name: upper_name(name),
        args,
    }
}

/// `port name : type`.
fn port(cx: &mut Cursor, node: &ast::PortDecl) -> Declaration {
    let name = node.lower_ident_token().expect(VALID);
    let before = cx.take(start(&name));
    let after = cx.take(start(&node.colon_token().expect(VALID)));
    let annotation = node.ty().expect(VALID);
    let before_type = cx.take(cx.range(annotation.syntax()).start());
    Declaration::PortAnnotation {
        name: c(Around { before, after }, lower_name(&name)),
        before_type,
        ty: ty(cx, &annotation),
    }
}

/// Elm 0.16's `port name = expr`.
fn legacy_port(cx: &mut Cursor, node: &ast::LegacyPortDecl) -> Declaration {
    let name = node.lower_ident_token().expect(VALID);
    let before = cx.take(start(&name));
    let after = cx.take(start(&node.eq_token().expect(VALID)));
    let value = node.expr().expect(VALID);
    let before_body = cx.take(cx.range(value.syntax()).start());
    Declaration::PortDefinition {
        name: c(Around { before, after }, lower_name(&name)),
        before_body,
        body: expr(cx, &value),
    }
}

/// The associativity whose keyword a fixity declaration has, and the
/// keyword.
fn associativity(keywords: [(Assoc, Option<SyntaxToken>); 3]) -> (Assoc, SyntaxToken) {
    keywords
        .into_iter()
        .find_map(|(assoc, keyword)| keyword.map(|k| (assoc, k)))
        .expect(VALID)
}

/// `infix left 6 (+) = add`.
fn infix(cx: &mut Cursor, node: &ast::InfixDecl) -> Declaration {
    let (assoc, keyword) = associativity([
        (Assoc::Left, node.left_token()),
        (Assoc::Right, node.right_token()),
        (Assoc::Non, node.non_token()),
    ]);
    let assoc = c(cx.take(start(&keyword)), assoc);
    let int = node.int_token().expect(VALID);
    let precedence = c(cx.take(start(&int)), int.text().parse().expect(VALID));
    let before = cx.take(start(&node.l_paren_token().expect(VALID)));
    let after = cx.take(start(&node.eq_token().expect(VALID)));
    let op = c(Around { before, after }, symbol(&node.op().expect(VALID)));
    let function = node.lower_ident_token().expect(VALID);
    let function = c(cx.take(start(&function)), lower_name(&function));
    Declaration::Fixity {
        assoc,
        precedence,
        op,
        function,
    }
}

/// Elm 0.18's `infixl 6 +`, with comments before the precedence and the
/// operator.
fn legacy_infix(cx: &mut Cursor, node: &ast::LegacyInfixDecl, assoc: Assoc) -> Declaration {
    let int = node.int_token().expect(VALID);
    let before_precedence = cx.take(start(&int));
    let op = node.op().expect(VALID);
    let before_op = cx.take(cx.range(op.syntax()).start());
    Declaration::LegacyFixity {
        assoc,
        before_precedence,
        precedence: int.text().parse().expect(VALID),
        before_op,
        op: symbol(&op),
    }
}

#[cfg(test)]
mod tests;
