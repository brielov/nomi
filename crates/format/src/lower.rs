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

use std::collections::{BTreeMap, BTreeSet};

use nomi_base::TextSize;
use nomi_syntax::ast::{self, AstNode};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, child_token, significant_tokens, tokens};

use crate::tree::{
    Around, C0Eol, C1, C2, Comment, Comments, Lower, Module, Namespace, Pair, SeparatorComments,
    Sequence, Symbol, TopLevelStructure, Upper, UpperRef, c,
};
use cursor::Cursor;
pub(crate) use literal::{is_number, is_text, negative, unsigned as literal_token};

const VALID: &str = "a validated tree has it";

/// The formatter tree of `root`, a module parsed from `text` that
/// validates for elm-format.
pub(crate) fn module(root: &SyntaxNode, text: &str) -> Module {
    let mut cx = Cursor::new(root, text);
    let module = ast::Module::cast(root.clone()).expect(VALID);
    let imports: Vec<ast::Import> = module.imports().collect();
    let text_end = TextSize::of(text);
    let body_start = module
        .decls()
        .next()
        .map_or(text_end, |d| cx.range(d.syntax()).start());

    let initial_comments = cx.take(match significant_tokens(root).next() {
        Some(first) => first.text_range().start(),
        None => fresh_line_end(root, text_end),
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
    let used = qualifiers(root);
    let mut merged = BTreeMap::new();
    let mut body = Vec::new();
    for (i, import) in imports.iter().enumerate() {
        let (name, method) = header::import(&mut cx, import);
        header::insert_import(&mut merged, &name, method, &used);
        match imports.get(i + 1) {
            Some(next) => comments.extend(cx.take(cx.range(next.syntax()).start())),
            None => comments_before(&mut cx, &mut body, body_start),
        }
    }

    for decl in module.decls() {
        declarations::declaration(&mut cx, &mut body, &decl);
    }
    // Upstream reads the comments after the last declaration as between
    // declarations too.
    comments_before(&mut cx, &mut body, text_end);
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
    tokens(root)
        .take_while(|t| t.text_range().end() <= end)
        .filter(|t| matches!(t.kind(), SyntaxKind::Newline | SyntaxKind::LineComment))
        .last()
        .map_or_else(TextSize::default, |t| t.text_range().end())
}

/// The comments before `end` onto `body`, which upstream reads as
/// comments between declarations.
fn comments_before<T>(cx: &mut Cursor, body: &mut Vec<TopLevelStructure<T>>, end: TextSize) {
    let comments = cx.take(end);
    body.extend(comments.into_iter().map(TopLevelStructure::BodyComment));
}

/// The qualifiers the module's names are written with, as `A.B`.
fn qualifiers(root: &SyntaxNode) -> BTreeSet<String> {
    root.descendants()
        .filter(|n| n.kind() == SyntaxKind::Qualifier)
        .map(|n| {
            let mut text = String::new();
            for token in significant_tokens(&n) {
                text.push_str(token.text());
            }
            text.truncate(text.trim_end_matches('.').len());
            text
        })
        .collect()
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

/// `node`'s child tokens of `kind`, among its own.
fn child_tokens(node: &SyntaxNode, kind: SyntaxKind) -> impl Iterator<Item = SyntaxToken> + use<> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .filter(move |t| t.kind() == kind)
}

/// The uppercase names among `node`'s children, as a module name or a
/// qualifier holds them.
fn upper_names(node: &SyntaxNode) -> Namespace {
    child_tokens(node, SyntaxKind::UpperIdent)
        .map(|t| upper_name(&t))
        .collect()
}

/// The modules a qualifier names; none without one.
fn namespace(qualifier: Option<ast::Qualifier>) -> Namespace {
    qualifier.map_or_else(Vec::new, |q| upper_names(q.syntax()))
}

/// The name in `node`, after the qualifier `node` may have.
fn upper_ref(node: &SyntaxNode, qualifier: Option<ast::Qualifier>) -> UpperRef {
    UpperRef {
        namespace: namespace(qualifier),
        name: upper_name(&child_token(node, SyntaxKind::UpperIdent).expect(VALID)),
    }
}

/// The value of an unqualified `True` or `False`, which upstream reads as
/// a literal.
fn bool_literal(namespace: &Namespace, name: &Upper) -> Option<bool> {
    if !namespace.is_empty() {
        return None;
    }
    match name.0.as_str() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

/// An operator, as its token spells it.
fn symbol(op: &ast::Op) -> Symbol {
    Symbol(
        significant_tokens(op.syntax())
            .next()
            .expect(VALID)
            .text()
            .to_owned(),
    )
}

/// Where each comma among the children of `node` starts.
fn commas(node: &SyntaxNode) -> Vec<TextSize> {
    child_tokens(node, SyntaxKind::Comma)
        .map(|t| start(&t))
        .collect()
}

/// `items`, each with the comments before it.
fn preceded<N: AstNode, T>(
    cx: &mut Cursor,
    items: impl Iterator<Item = N>,
    mut lower: impl FnMut(&mut Cursor, &N) -> T,
) -> Vec<C1<T>> {
    items
        .map(|item| {
            let comments = cx.take(cx.range(item.syntax()).start());
            c(comments, lower(cx, &item))
        })
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
            let before = cx.take(cx.range(item.syntax()).start());
            let value = lower(cx, &item);
            let after = cx.take(commas.get(i).copied().unwrap_or_else(|| start(close)));
            c(Around { before, after }, value)
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
        let after_separator = cx.take(cx.range(item.syntax()).start());
        let value = lower(cx, &item);
        let eol = cx.eol(cx.range(item.syntax()));
        let next = commas.get(i).copied().unwrap_or_else(|| start(close));
        let before_next = cx.take(next);
        let comments = SeparatorComments {
            before_separator: std::mem::replace(&mut leading, before_next),
            after_separator,
            eol,
        };
        terms.push(c(comments, value));
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
) -> Pair<T> {
    let name = child_token(field, SyntaxKind::LowerIdent).expect(VALID);
    let separator = child_token(field, SyntaxKind::Colon)
        .or_else(|| child_token(field, SyntaxKind::Eq))
        .expect(VALID);
    let key = c(cx.take(start(&separator)), lower_name(&name));
    let value = c(cx.take(cx.range(value.syntax()).start()), lower(cx, value));
    Pair {
        key,
        value,
        force_multiline: cx.force_multiline(field),
    }
}

/// `terms` with `separators` between them, like `a -> b -> c`: each term
/// with the `--` comment that may end its line, and the comments around
/// the separator before it; upstream's `separated`.
fn separated<N: AstNode, T>(
    cx: &mut Cursor,
    terms: Vec<N>,
    separators: &[SyntaxToken],
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
            let before_separator = cx.take(start(separator));
            let after_separator = cx.take(cx.range(node.syntax()).start());
            let (value, eol) = term(cx, node);
            let comments = SeparatorComments {
                before_separator,
                after_separator,
                eol,
            };
            c(comments, value)
        })
        .collect();
    (c(eol, first), Sequence(rest))
}

/// `node` with the `--` comment ending its line, unless it spans lines;
/// upstream's `withEol`.
fn with_eol<N: AstNode, T>(
    cx: &mut Cursor,
    node: &N,
    lower: impl FnOnce(&mut Cursor, &N) -> T,
) -> C0Eol<T> {
    let value = lower(cx, node);
    c(cx.eol(cx.range(node.syntax())), value)
}

/// An item of a parenthesized list, its `--` comment first among those
/// after it; upstream's `spaceySepBy1''`.
fn eol_after<T>(item: C2<C0Eol<T>>) -> C2<T> {
    let (mut around, inner) = (item.comments, item.value);
    if let Some(eol) = inner.comments {
        around.after.insert(0, Comment::Line(eol));
    }
    c(around, inner.value)
}

/// `item` in `parens`, or without them when they hold no comments, as
/// upstream drops them.
fn parens_or_bare<T>(item: C2<T>, parens: impl FnOnce(Box<C2<T>>) -> T) -> T {
    if item.comments.is_empty() {
        item.value
    } else {
        parens(Box::new(item))
    }
}

/// What `lower` makes of the fragment `text` holds, for rendering's
/// tests; it must parse and place every comment.
#[cfg(test)]
fn fragment<F: AstNode, N, T>(
    text: &str,
    parse: fn(&str) -> nomi_syntax::Parse,
    node: fn(F) -> Option<N>,
    lower: fn(&mut Cursor, &N) -> T,
) -> T {
    let parse = parse(text);
    assert!(parse.errors().is_empty(), "{:?}", parse.errors());
    let root = parse.syntax();
    let mut cx = Cursor::new(&root, text);
    let node = node(F::cast(root).expect(VALID)).expect(VALID);
    let lowered = lower(&mut cx, &node);
    cx.finish();
    lowered
}

/// The type `text` holds.
#[cfg(test)]
pub(crate) fn type_fragment(text: &str) -> crate::tree::Type {
    fragment(
        text,
        nomi_syntax::parse_type,
        |f: ast::TypeFragment| f.ty(),
        types::ty,
    )
}

/// The pattern `text` holds.
#[cfg(test)]
pub(crate) fn pattern_fragment(text: &str) -> crate::tree::Pattern {
    fragment(
        text,
        nomi_syntax::parse_pattern,
        |f: ast::PatternFragment| f.pattern(),
        patterns::pattern,
    )
}

#[cfg(test)]
mod tests;
