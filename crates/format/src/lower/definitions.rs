//! Annotations and definitions, at the top level and in a `let`; after
//! upstream's `typeAnnotation` and `definition` in `Parse/Expression.hs`.

use nomi_syntax::SyntaxToken;
use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::expressions::expr;
use super::patterns::pattern;
use super::types::ty;
use super::{VALID, lower_name, preceded, start, symbol};
use crate::tree::{AnnotatedName, C1, Comments, CommonDeclaration, Pattern, Symbol, c};

/// A value's annotation and its definition, each to `part` with the
/// comments before it. Upstream reads an annotation on its own, so either
/// may be missing.
pub(super) fn value_parts(
    cx: &mut Cursor,
    signature: Option<ast::Signature>,
    def: Option<ast::Definition>,
    mut part: impl FnMut(Comments, CommonDeclaration),
) {
    if let Some(signature) = signature {
        let comments = cx.take(cx.range(signature.syntax()).start());
        part(comments, annotation(cx, &signature));
    }
    if let Some(def) = def {
        let comments = cx.take(cx.range(def.syntax()).start());
        part(comments, definition(cx, &def));
    }
}

/// `name : type`, with the comments before `:` and after it.
fn annotation(cx: &mut Cursor, node: &ast::Signature) -> CommonDeclaration {
    let name = match node.lower_ident_token() {
        Some(name) => AnnotatedName::Var(lower_name(&name)),
        None => AnnotatedName::Op(operator(node.operator_name())),
    };
    let before_colon = cx.take(start(&node.colon_token().expect(VALID)));
    let annotation = node.ty().expect(VALID);
    let before_type = cx.take(cx.range(annotation.syntax()).start());
    CommonDeclaration::TypeAnnotation {
        name: c(before_colon, name),
        ty: c(before_type, ty(cx, &annotation)),
    }
}

/// `name args = body`; a name, unlike a pattern, takes arguments.
fn definition(cx: &mut Cursor, node: &ast::Definition) -> CommonDeclaration {
    let name = match node.lower_ident_token() {
        Some(name) => Pattern::Var(lower_name(&name)),
        None => Pattern::Op(operator(node.operator_name())),
    };
    let args = preceded(cx, node.patterns(), pattern);
    let eq = node.eq_token().expect(VALID);
    body(cx, name, args, &eq, &node.expr().expect(VALID))
}

/// `pattern = body`.
pub(super) fn destructure(
    cx: &mut Cursor,
    target: &ast::Pattern,
    eq: &SyntaxToken,
    value: &ast::Expr,
) -> CommonDeclaration {
    let target = pattern(cx, target);
    body(cx, target, Vec::new(), eq, value)
}

/// The rest of a definition, from the comments before `=`, which upstream
/// joins with those after it.
fn body(
    cx: &mut Cursor,
    name: Pattern,
    args: Vec<C1<Pattern>>,
    eq: &SyntaxToken,
    value: &ast::Expr,
) -> CommonDeclaration {
    let mut before_body = cx.take(start(eq));
    before_body.extend(cx.take(cx.range(value.syntax()).start()));
    CommonDeclaration::Definition {
        name,
        args,
        before_body,
        body: expr(cx, value),
    }
}

/// The operator of `(op)`.
fn operator(name: Option<ast::OperatorName>) -> Symbol {
    symbol(&name.and_then(|n| n.op()).expect(VALID))
}
