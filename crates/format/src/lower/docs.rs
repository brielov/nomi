//! Doc comments: their Markdown, and the Elm code in its code blocks,
//! which upstream's `formatDocComment` reads while rendering.

use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{
    Parse, SyntaxNode, SyntaxToken, Tool, parse_declarations, parse_expressions, parse_module,
    validate,
};

use super::cursor::{self, Cursor};
use super::{VALID, comments_before, declarations, expressions};
use crate::markdown::{self, CodeAttr};
use crate::tree::{C0Eol, Declaration, Docs, ElmCode, Expr, TopLevelStructure, c};

/// The doc comment `token`: the Markdown between `{-|` and `-}`.
pub(super) fn docs(token: &SyntaxToken) -> Docs {
    let text = cursor::block_body(&token.text()[3..]);
    Docs {
        blocks: markdown::parse(&text, &mut elm_code),
        source: token.text().to_owned(),
    }
}

/// The Elm in a code block marked `elm` or not at all: declarations,
/// else expressions, else a module, as the first that elm-format
/// parses.
fn elm_code(attr: &CodeAttr, text: &str) -> Option<ElmCode> {
    if !attr.is_elm() {
        return None;
    }
    let valid =
        |parse: &Parse| parse.errors().is_empty() && validate(parse, Tool::ElmFormat).is_empty();
    let parse = parse_declarations(text);
    if valid(&parse) {
        return Some(ElmCode::Declarations(declarations_fragment(
            &parse.syntax(),
            text,
        )));
    }
    let parse = parse_expressions(text);
    if valid(&parse) {
        return Some(ElmCode::Expressions(expressions_fragment(
            &parse.syntax(),
            text,
        )));
    }
    let parse = parse_module(text);
    valid(&parse).then(|| ElmCode::Module(Box::new(super::module(&parse.syntax(), text))))
}

/// Declarations, and the comments before each, as a module's body;
/// upstream's `topLevel`. Validation leaves no comment after the last.
fn declarations_fragment(root: &SyntaxNode, text: &str) -> Vec<TopLevelStructure<Declaration>> {
    let mut cx = Cursor::new(root, text);
    let mut body = Vec::new();
    for decl in ast::DeclarationsFragment::cast(root.clone())
        .expect(VALID)
        .decls()
    {
        declarations::declaration(&mut cx, &mut body, &decl);
    }
    cx.finish();
    body
}

/// Expressions, each with the `--` comment ending its line, and the
/// comments before each; upstream's `topLevel (withEol expr)`.
fn expressions_fragment(root: &SyntaxNode, text: &str) -> Vec<TopLevelStructure<C0Eol<Expr>>> {
    let mut cx = Cursor::new(root, text);
    let mut body = Vec::new();
    for expr in ast::ExpressionsFragment::cast(root.clone())
        .expect(VALID)
        .exprs()
    {
        let expr_range = cx.range(expr.syntax());
        comments_before(&mut cx, &mut body, expr_range.start());
        let value = expressions::expr(&mut cx, &expr);
        let eol = cx.eol(expr_range);
        body.push(TopLevelStructure::Entry(c(eol, value)));
    }
    cx.finish();
    body
}
