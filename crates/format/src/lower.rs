//! Lowering: the formatter tree from a validated syntax tree.
//!
//! The syntax tree keeps comments as trivia between tokens; the formatter
//! tree keeps them where elm-format's parser attaches them. One cursor
//! walks the comments forward, and each slot takes those before the token
//! upstream's parser reads next, so every comment lands once, in order.
//!
//! A validated tree has every child lowering asks for, so a missing one is
//! a bug and panics.

mod cursor;
mod header;

use std::collections::BTreeMap;

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::tree::{Commented, Docs, Module, TopLevelStructure};
use cursor::Cursor;

const VALID: &str = "a validated tree has it";

/// The formatter tree of `root`, a module parsed from `text` that
/// validates for elm-format.
pub(crate) fn module(root: &SyntaxNode, text: &str) -> Module {
    let mut cx = Cursor::new(root, text);
    let module = ast::Module::cast(root.clone()).expect(VALID);
    let imports: Vec<ast::Import> = module.imports().collect();
    let body_start = module
        .decls()
        .next()
        .map_or(text_end(text), |d| range(d.syntax()).start());

    let first = significant(root).next();
    let initial_comments = cx.take(first.map_or(text_end(text), |t| t.text_range().start()));
    let header = module.module_header().map(|h| header::header(&mut cx, &h));

    // Upstream reads the comments before the docs, those after, and those
    // between imports, into the comments before the imports.
    let mut comments = Vec::new();
    let docs = module.doc_comment_token().map(|token| {
        comments.extend(cx.take(token.text_range().start()));
        Docs(cursor::block_body(&token.text()[3..]))
    });
    comments.extend(
        cx.take(
            imports
                .first()
                .map_or(body_start, |i| range(i.syntax()).start()),
        ),
    );
    let mut merged = BTreeMap::new();
    let mut body = Vec::new();
    for (i, import) in imports.iter().enumerate() {
        let (name, method) = header::import(&mut cx, import);
        header::insert_with(&mut merged, name, method, header::merge_import);
        match imports.get(i + 1) {
            Some(next) => comments.extend(cx.take(range(next.syntax()).start())),
            None => body.extend(
                cx.take(body_start)
                    .into_iter()
                    .map(TopLevelStructure::BodyComment),
            ),
        }
    }

    // Declarations lower from plan 2.7; until then their comments go
    // unused.
    let _ = cx.take(text_end(text));
    cx.finish();
    Module {
        initial_comments,
        header,
        docs,
        imports: c(comments, merged),
        body,
    }
}

fn c<C, T>(comments: C, value: T) -> Commented<C, T> {
    Commented { comments, value }
}

fn text_end(text: &str) -> TextSize {
    TextSize::of(text)
}

/// The tokens of `node` that are not trivia.
fn significant(node: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| !t.kind().is_trivia())
}

/// From the first to the last token of `node` that is not trivia; a
/// node's own range takes in the trivia before it.
fn range(node: &SyntaxNode) -> TextRange {
    let mut tokens = significant(node);
    let first = tokens.next().expect(VALID).text_range();
    let last = tokens.last().map_or(first, |t| t.text_range());
    first.cover(last)
}

/// The child token of `node` of `kind`.
fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
}

#[cfg(test)]
mod tests;
