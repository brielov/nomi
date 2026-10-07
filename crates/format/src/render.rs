//! Rendering: the formatter tree as elm-format lays it out, a port of
//! `ElmFormat/Render/Box.hs`.
//!
//! Each function builds the block its upstream namesake does, from the
//! same parts in the same order, so the two can be read side by side.

mod docs;
mod header;

use crate::block::{Block, Line};
use crate::structure::{forceable_space_sep_or_stack1, space_sep_or_indented, space_sep_or_stack};
use crate::tree::{Comment, Module, TopLevelStructure};

/// Blank lines between top-level parts; upstream renders a module with
/// `formatModule elmVersion True 2`.
const SPACING: usize = 2;

/// `module` as elm-format prints it. Declarations render from plan 2.11;
/// until then `body`, the source text from the first body item to the
/// end, stands in for them.
pub(crate) fn module(module: &Module, body: &str) -> String {
    let mut blocks = Vec::new();
    if !module.initial_comments.is_empty() {
        blocks.extend(module.initial_comments.iter().map(comment));
        blocks.extend([Block::blank_line(), Block::blank_line()]);
    }
    blocks.extend(header::module_header(module));
    let space_before_body = match module.body.first() {
        None => 0,
        Some(TopLevelStructure::BodyComment(_)) => SPACING + 1,
        Some(_) => SPACING,
    };
    blocks.extend(std::iter::repeat_with(Block::blank_line).take(space_before_body));
    let mut out = Block::stack1(blocks).render();
    out.push_str(body);
    out
}

/// `(inner)`; upstream's `parens`.
fn parens(inner: Block) -> Block {
    match inner {
        Block::SingleLine(line) => Block::line(Line::row([Line::text("("), line, Line::text(")")])),
        inner => Block::stack1([inner.prefix(Line::text("(")), Block::line(Line::text(")"))]),
    }
}

/// A lowercase or uppercase name, with Elm 0.19's `_` for `'`; upstream's
/// `formatVarName'`.
fn name(name: &str) -> Line {
    Line::text(name.replace('\'', "_"))
}

/// `A.B.C`; upstream's `formatQualifiedUppercaseIdentifier`.
fn qualified(names: &[crate::tree::Upper]) -> Line {
    let names: Vec<String> = names.iter().map(|n| n.0.replace('\'', "_")).collect();
    Line::text(names.join("."))
}

fn comment(comment: &Comment) -> Block {
    match comment {
        Comment::Block(lines) => match &lines[..] {
            [] => Block::line(Line::text("{- -}")),
            [line] => Block::line(Line::row([
                Line::text("{-"),
                Line::space(),
                Line::text(line.as_str()),
                Line::space(),
                Line::text("-}"),
            ])),
            lines => Block::stack1([
                Block::stack1(lines.iter().map(|l| Block::line(Line::text(l.as_str()))))
                    .prefix(Line::row([Line::text("{-"), Line::space()])),
                Block::line(Line::text("-}")),
            ]),
        },
        Comment::Line(text) => {
            Block::must_break(Line::row([Line::text("--"), Line::text(text.as_str())]))
        }
        Comment::TrickOpener => Block::must_break(Line::text("{--}")),
        Comment::TrickCloser => Block::must_break(Line::text("--}")),
        Comment::TrickBlock(text) => Block::must_break(Line::row([
            Line::text("{--"),
            Line::text(text.as_str()),
            Line::text("-}"),
        ])),
    }
}

/// Upstream's `formatComments`.
fn comments(comments: &[Comment]) -> Option<Block> {
    let (first, rest) = comments.split_first()?;
    Some(space_sep_or_stack(
        comment(first),
        rest.iter().map(comment).collect(),
    ))
}

/// `inner` between the comments before and after it; upstream's
/// `formatCommented`.
fn commented(pre: &[Comment], inner: Block, post: &[Comment]) -> Block {
    forceable_space_sep_or_stack1(
        false,
        comments(pre)
            .into_iter()
            .chain([inner])
            .chain(comments(post))
            .collect(),
    )
}

/// Upstream's `formatPreCommented`.
fn pre_commented(pre: &[Comment], inner: Block) -> Block {
    commented(pre, inner, &[])
}

/// Upstream's `formatTailCommented`.
fn tail_commented(inner: Block, post: &[Comment]) -> Block {
    commented(&[], inner, post)
}

/// `word value`, with comments around `word`; upstream's
/// `formatKeywordCommented`.
fn keyword_commented(word: &str, pre: &[Comment], post: &[Comment], value: Block) -> Block {
    space_sep_or_indented(
        commented(pre, Block::line(Line::text(word)), post),
        vec![value],
    )
}

/// Haskell's `lines`: no last line after a final newline.
fn lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// A doc comment of `docs`, already rendered; upstream's
/// `formatDocCommentString`.
fn doc_comment_string(docs: &str) -> Block {
    let close = Block::line(Line::text("-}"));
    match &lines(docs)[..] {
        [] => Block::line(Line::row([
            Line::text("{-|"),
            Line::space(),
            Line::text("-}"),
        ])),
        [first, rest @ ..] => Block::line(Line::row([
            Line::text("{-|"),
            Line::space(),
            Line::text(*first),
        ]))
        .and_then(rest.iter().map(|l| Block::line(Line::text(*l))))
        .and_then([close]),
    }
}

#[cfg(test)]
mod tests;
