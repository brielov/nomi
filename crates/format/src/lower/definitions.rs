//! Annotations and definitions, at the top level and in a `let`; after
//! upstream's `typeAnnotation` and `definition` in `Parse/Expression.hs`.

use nomi_syntax::SyntaxToken;
use nomi_syntax::ast::{self, AstNode};

use super::cursor::Cursor;
use super::expressions::expr;
use super::patterns::pattern;
use super::types::ty;
use super::{VALID, c, lower_name, start, symbol};
use crate::tree::{C1, CommonDeclaration, Pattern, Ref, Symbol};

/// `name : type`, with the comments before `:` and after it.
pub(super) fn annotation(cx: &mut Cursor, node: &ast::Signature) -> CommonDeclaration {
    let name = match node.lower_ident_token() {
        Some(name) => Ref::Var((), lower_name(&name)),
        None => Ref::Op(operator(node.operator_name())),
    };
    let before_colon = cx.take(start(&node.colon_token().expect(VALID)));
    let annotation = node.ty().expect(VALID);
    let before_type = cx.take(cx.range(annotation.syntax()).start());
    CommonDeclaration::TypeAnnotation(c(before_colon, name), c(before_type, ty(cx, &annotation)))
}

/// `name args = body`; a name, unlike a pattern, takes arguments.
pub(super) fn definition(cx: &mut Cursor, node: &ast::Definition) -> CommonDeclaration {
    let name = match node.lower_ident_token() {
        Some(name) => Pattern::Var(lower_name(&name)),
        None => Pattern::Op(operator(node.operator_name())),
    };
    let args = node
        .patterns()
        .map(|arg| {
            let comments = cx.take(cx.range(arg.syntax()).start());
            c(comments, pattern(cx, &arg))
        })
        .collect();
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
    let mut comments = cx.take(start(eq));
    comments.extend(cx.take(cx.range(value.syntax()).start()));
    CommonDeclaration::Definition(name, args, comments, expr(cx, value))
}

/// The operator of `(op)`.
fn operator(name: Option<ast::OperatorName>) -> Symbol {
    symbol(&name.and_then(|n| n.op()).expect(VALID))
}
