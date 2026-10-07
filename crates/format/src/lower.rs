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
mod literal;
mod patterns;
mod types;

use std::collections::BTreeMap;

use nomi_base::{TextRange, TextSize};
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::tree::{
    C0Eol, C2, Commented, Docs, Lower, Module, Namespace, Sequence, TopLevelStructure, Upper,
    UpperRef,
};
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

    // Declarations lower from plan 2.7; until then the comments between
    // their types and patterns go unused, and lowering those on their own
    // checks them over every module.
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        if let Some(ty) = ast::Type::cast(node.clone()) {
            let _ = cx.take(range(&node).start());
            types::ty(&mut cx, &ty);
        } else if let Some(pattern) = ast::Pattern::cast(node.clone()) {
            let _ = cx.take(range(&node).start());
            patterns::pattern(&mut cx, &pattern);
        } else {
            let children: Vec<_> = node.children().collect();
            stack.extend(children.into_iter().rev());
        }
    }
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

fn start(token: &SyntaxToken) -> TextSize {
    token.text_range().start()
}

fn lower_name(token: &SyntaxToken) -> Lower {
    Lower(token.text().to_owned())
}

fn upper_name(token: &SyntaxToken) -> Upper {
    Upper(token.text().to_owned())
}

/// The name in `node`, after the qualifier `node` may have.
fn upper_ref(node: &SyntaxNode, qualifier: Option<ast::Qualifier>) -> UpperRef {
    let namespace: Namespace = qualifier.map_or_else(Vec::new, |q| {
        q.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::UpperIdent)
            .map(|t| upper_name(&t))
            .collect()
    });
    (
        namespace,
        upper_name(&token(node, SyntaxKind::UpperIdent).expect(VALID)),
    )
}

/// Where each comma among the children of `node` starts.
fn commas(node: &SyntaxNode) -> Vec<TextSize> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(|t| t.kind() == SyntaxKind::Comma)
        .map(|t| start(&t))
        .collect()
}

/// The comma-separated `items` of `node`, each with the comments before
/// it and those before the comma or `close` after it; upstream's
/// `surround''`.
fn surround<N: AstNode, T>(
    cx: &mut Cursor,
    node: &SyntaxNode,
    items: impl Iterator<Item = N>,
    close: &SyntaxToken,
    mut lower: impl FnMut(&mut Cursor, &N) -> T,
) -> Vec<C2<T>> {
    let commas = commas(node);
    items
        .enumerate()
        .map(|(i, item)| {
            let pre = cx.take(range(item.syntax()).start());
            let value = lower(cx, &item);
            let post = cx.take(commas.get(i).copied().unwrap_or_else(|| start(close)));
            c((pre, post), value)
        })
        .collect()
}

/// `terms` with `separators` between them, like `a -> b -> c`: each term
/// with the `--` comment that may end its line, and the comments around
/// the separator before it; upstream's `separated`.
fn separated<N: AstNode, T>(
    cx: &mut Cursor,
    terms: Vec<N>,
    separators: Vec<SyntaxToken>,
    mut lower: impl FnMut(&mut Cursor, &N) -> T,
) -> (C0Eol<T>, Sequence<T>) {
    let mut terms = terms.into_iter();
    let mut term = |cx: &mut Cursor, node: N| {
        let value = lower(cx, &node);
        (value, cx.rest_of_line(range(node.syntax()).end()))
    };
    let (first, eol) = term(cx, terms.next().expect(VALID));
    let rest = separators
        .iter()
        .zip(terms)
        .map(|(separator, node)| {
            let pre = cx.take(start(separator));
            let post = cx.take(range(node.syntax()).start());
            let (value, eol) = term(cx, node);
            c((pre, post, eol), value)
        })
        .collect();
    (c(eol, first), Sequence(rest))
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
