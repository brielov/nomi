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
mod declarations;
mod definitions;
mod docs;
mod expressions;
mod header;
mod literal;
mod patterns;
mod types;

use std::collections::BTreeMap;

use nomi_base::TextSize;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::tree::{
    C0Eol, C2, Commented, Comments, ForceMultiline, Lower, Module, Namespace, Pair, Sequence,
    Symbol, TopLevelStructure, Upper, UpperRef,
};
use cursor::Cursor;
pub(crate) use literal::{negative, unsigned as literal_token};

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
        .map_or(text_end(text), |d| cx.range(d.syntax()).start());

    let initial_comments = cx.take(match significant(root).next() {
        Some(first) => first.text_range().start(),
        None => fresh_line_end(root, text_end(text)),
    });
    let header = module.module_header().map(|h| header::header(&mut cx, &h));

    // Upstream reads the comments before the docs, those after, and those
    // between imports, into the comments before the imports.
    let mut comments = Vec::new();
    let docs = module.doc_comment_token().map(|token| {
        comments.extend(cx.take(token.text_range().start()));
        docs::docs(&token)
    });
    comments.extend(cx.take(match imports.first() {
        Some(import) => cx.range(import.syntax()).start(),
        None => fresh_line_end(root, body_start),
    }));
    let mut merged = BTreeMap::new();
    let mut body = Vec::new();
    for (i, import) in imports.iter().enumerate() {
        let (name, method) = header::import(&mut cx, import);
        header::insert_with(&mut merged, name, method, header::merge_import);
        match imports.get(i + 1) {
            Some(next) => comments.extend(cx.take(cx.range(next.syntax()).start())),
            None => body.extend(
                cx.take(body_start)
                    .into_iter()
                    .map(TopLevelStructure::BodyComment),
            ),
        }
    }

    for decl in module.decls() {
        declarations::declaration(&mut cx, &mut body, &decl);
    }
    // Upstream reads the comments after the last declaration as between
    // declarations too.
    body.extend(
        cx.take(text_end(text))
            .into_iter()
            .map(TopLevelStructure::BodyComment),
    );
    cx.finish();
    Module {
        initial_comments,
        header,
        docs,
        imports: c(comments, merged),
        body,
    }
}

/// Where upstream's `freshLine`, reading the comments before `end`, stops:
/// after the last line end, which a `--` comment counts as. The comments
/// after it, ending a module's text, are the body's.
fn fresh_line_end(root: &SyntaxNode, end: TextSize) -> TextSize {
    root.descendants_with_tokens()
        .filter_map(|e| e.into_token())
        .take_while(|t| t.text_range().end() <= end)
        .filter(|t| matches!(t.kind(), SyntaxKind::Newline | SyntaxKind::LineComment))
        .last()
        .map_or(TextSize::default(), |t| t.text_range().end())
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

/// The modules a qualifier names; none without one.
fn namespace(qualifier: Option<ast::Qualifier>) -> Namespace {
    qualifier.map_or_else(Vec::new, |q| {
        q.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.kind() == SyntaxKind::UpperIdent)
            .map(|t| upper_name(&t))
            .collect()
    })
}

/// The name in `node`, after the qualifier `node` may have.
fn upper_ref(node: &SyntaxNode, qualifier: Option<ast::Qualifier>) -> UpperRef {
    (
        namespace(qualifier),
        upper_name(&token(node, SyntaxKind::UpperIdent).expect(VALID)),
    )
}

/// An operator, as its token spells it.
fn symbol(op: &ast::Op) -> Symbol {
    Symbol(
        significant(op.syntax())
            .next()
            .expect(VALID)
            .text()
            .to_owned(),
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
            let pre = cx.take(cx.range(item.syntax()).start());
            let value = lower(cx, &item);
            let post = cx.take(commas.get(i).copied().unwrap_or_else(|| start(close)));
            c((pre, post), value)
        })
        .collect()
}

/// The comma-separated `items` of `node`, each with the comments before
/// the comma before it, after that comma, and ending its line, and the
/// comments before `close`; upstream's `sectionedGroup`.
fn sectioned<N: AstNode, T>(
    cx: &mut Cursor,
    node: &SyntaxNode,
    items: impl Iterator<Item = N>,
    close: &SyntaxToken,
    mut lower: impl FnMut(&mut Cursor, &N) -> T,
) -> (Sequence<T>, Comments) {
    let commas = commas(node);
    let mut leading = Vec::new();
    let mut terms = Vec::new();
    for (i, item) in items.enumerate() {
        let pre = cx.take(cx.range(item.syntax()).start());
        let value = lower(cx, &item);
        let eol = cx.eol(cx.range(item.syntax()));
        let next = commas.get(i).copied().unwrap_or_else(|| start(close));
        let before_next = cx.take(next);
        terms.push(c(
            (std::mem::replace(&mut leading, before_next), pre, eol),
            value,
        ));
    }
    // With no items, upstream reads the comments inside as trailing.
    let trailing = if terms.is_empty() {
        cx.take(start(close))
    } else {
        leading
    };
    (Sequence(terms), trailing)
}

/// `name : value` in `field`, or `name = value`; upstream's `pair`, which
/// takes either separator.
fn pair<N: AstNode, T>(
    cx: &mut Cursor,
    field: &SyntaxNode,
    value: &N,
    lower: impl FnOnce(&mut Cursor, &N) -> T,
) -> Pair<Lower, T> {
    let name = token(field, SyntaxKind::LowerIdent).expect(VALID);
    let separator = token(field, SyntaxKind::Colon)
        .or_else(|| token(field, SyntaxKind::Eq))
        .expect(VALID);
    let key = c(cx.take(start(&separator)), lower_name(&name));
    let value = c(cx.take(cx.range(value.syntax()).start()), lower(cx, value));
    Pair {
        key,
        value,
        force_multiline: ForceMultiline(cx.multiline(cx.range(field))),
    }
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
        (value, cx.rest_of_line(cx.range(node.syntax()).end()))
    };
    let (first, eol) = term(cx, terms.next().expect(VALID));
    let rest = separators
        .iter()
        .zip(terms)
        .map(|(separator, node)| {
            let pre = cx.take(start(separator));
            let post = cx.take(cx.range(node.syntax()).start());
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

/// The child token of `node` of `kind`.
fn token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| t.kind() == kind)
}

/// The type `text` holds, for rendering's tests; it must parse and place
/// every comment.
#[cfg(test)]
pub(crate) fn type_fragment(text: &str) -> crate::tree::Type {
    let parse = nomi_syntax::parse_type(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let root = parse.syntax();
    let mut cx = Cursor::new(&root, text);
    let node = ast::TypeFragment::cast(root)
        .expect(VALID)
        .ty()
        .expect(VALID);
    let lowered = types::ty(&mut cx, &node);
    cx.finish();
    lowered
}

/// The pattern `text` holds, for rendering's tests; it must parse and
/// place every comment.
#[cfg(test)]
pub(crate) fn pattern_fragment(text: &str) -> crate::tree::Pattern {
    let parse = nomi_syntax::parse_pattern(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let root = parse.syntax();
    let mut cx = Cursor::new(&root, text);
    let node = ast::PatternFragment::cast(root)
        .expect(VALID)
        .pattern()
        .expect(VALID);
    let lowered = patterns::pattern(&mut cx, &node);
    cx.finish();
    lowered
}

#[cfg(test)]
mod tests;
