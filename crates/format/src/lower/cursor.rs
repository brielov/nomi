//! The comments of a module, handed out in order.
//!
//! elm-format's parser keeps each comment where the whitespace parser
//! that read it puts it. Lowering does the same with one cursor: each
//! comment slot takes the comments before the next token its parser
//! reads, so the comments go out in source order, each once.

use nomi_base::{TextRange, TextSize};
use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::block::is_space;
use crate::tree::{Comment, Comments};

#[derive(Debug)]
pub(super) struct Cursor<'a> {
    text: &'a str,
    /// Line and block comments, in order.
    comments: Vec<SyntaxToken>,
    next: usize,
    /// Where each newline starts, in order.
    newlines: Vec<TextSize>,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(root: &SyntaxNode, text: &'a str) -> Cursor<'a> {
        let mut comments = Vec::new();
        let mut newlines = Vec::new();
        for token in root
            .descendants_with_tokens()
            .filter_map(|e| e.into_token())
        {
            match token.kind() {
                SyntaxKind::LineComment | SyntaxKind::BlockComment => comments.push(token),
                SyntaxKind::Newline => newlines.push(token.text_range().start()),
                _ => {}
            }
        }
        Cursor {
            text,
            comments,
            next: 0,
            newlines,
        }
    }

    /// The comments before `end` not yet taken.
    pub(super) fn take(&mut self, end: TextSize) -> Comments {
        let mut taken = Vec::new();
        while let Some(token) = self.comments.get(self.next) {
            if token.text_range().start() >= end {
                break;
            }
            taken.push(comment(token));
            self.next += 1;
        }
        taken
    }

    /// The `--` comment ending the line of `item`, if only spaces come
    /// between; upstream's `withEol`, which reads none after an item that
    /// spans lines.
    pub(super) fn eol(&mut self, item: TextRange) -> Option<String> {
        if self.multiline(item) {
            return None;
        }
        let token = self.comments.get(self.next)?;
        let range = token.text_range();
        let between = &self.text[TextRange::new(item.end(), range.start())];
        if token.kind() != SyntaxKind::LineComment || !between.bytes().all(|b| b == b' ') {
            return None;
        }
        self.next += 1;
        Some(token.text()[2..].to_owned())
    }

    /// Whether a newline outside block comments falls in `range`;
    /// upstream's `trackNewline`.
    pub(super) fn multiline(&self, range: TextRange) -> bool {
        let first = self.newlines.partition_point(|&n| n < range.start());
        self.newlines.get(first).is_some_and(|&n| n < range.end())
    }

    /// Checks every comment was taken.
    pub(super) fn finish(self) {
        debug_assert!(
            self.next == self.comments.len(),
            "comments never lowered: {:?}",
            &self.comments[self.next..]
        );
    }
}

/// A line or block comment, as upstream's `lineComment` and
/// `multiComment` read it.
fn comment(token: &SyntaxToken) -> Comment {
    let text = token.text();
    if token.kind() == SyntaxKind::LineComment {
        return line_comment(&text[2..]);
    }
    if text == "{--}" {
        return Comment::TrickOpener;
    }
    match text[2..].strip_prefix('-') {
        Some(_) => Comment::TrickBlock(block_body(&text[3..])),
        None => Comment::Block(trim_indent(lines(&block_body(&text[2..])))),
    }
}

/// A line comment's text after `--`.
fn line_comment(text: &str) -> Comment {
    match text.strip_prefix('}') {
        Some(rest) if rest.bytes().all(|b| b == b' ') => Comment::TrickCloser,
        _ => Comment::Line(text.to_owned()),
    }
}

/// A block comment's text between its opening and `-}`, without the
/// spaces at either end; upstream's `closeComment`, after the spaces the
/// opening skips.
pub(super) fn block_body(text: &str) -> String {
    let text = text.strip_suffix("-}").expect("the lexer closes comments");
    text.trim_start_matches(' ')
        .trim_end_matches(' ')
        .to_owned()
}

/// Haskell's `lines`: no last line after a final newline.
fn lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Removes from each line after the first the indentation they all
/// share, ignoring blank lines; upstream's `trimIndent`.
fn trim_indent(mut lines: Vec<String>) -> Vec<String> {
    let depth = lines
        .iter()
        .skip(1)
        .filter_map(|line| {
            let indent = line.chars().take_while(|&c| is_space(c)).count();
            (indent != line.chars().count()).then_some(indent)
        })
        .min()
        .unwrap_or(0);
    for line in lines.iter_mut().skip(1) {
        let cut = line
            .char_indices()
            .nth(depth)
            .map_or(line.len(), |(i, _)| i);
        line.drain(..cut);
    }
    lines
}
