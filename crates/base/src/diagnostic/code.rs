//! Source snippets: a port of `Reporting/Render/Code.hs`. Documents name
//! the code they show by byte range, so a diagnostic doesn't hold its
//! file's text; [`expand`] puts the lines in, numbered and marked as the
//! release does, before a document is laid out.
//!
//! Releases keep positions differently, and it shows: 0.19.1 counts rows
//! and columns from 1 in 16 bits, later releases from 0. So a pair on one
//! row is numbered one less from 0.19.2 on, while every other snippet is
//! numbered from 1.

use std::fmt;

use text_size::TextRange;

use super::doc::{Doc, OneLine, Pair, TwoSnippets, framed, red, text, vcat};
use crate::{ColumnUnit, Compat, LineIndex, ghc};

/// A file's text, split into lines as upstream splits it to show them
/// (`Code.Source`).
#[derive(Clone, Debug)]
pub struct Source<'a> {
    text: &'a str,
    index: LineIndex,
    /// Haskell's `lines`, which splits at LF only and so leaves the CR of
    /// a CRLF in its line, then an empty line.
    lines: Vec<&'a str>,
}

/// A row and column as the release keeps them (`A.Position`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub row: u32,
    pub col: u32,
}

/// A region as the release keeps it (`A.Region`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub start: Position,
    pub end: Position,
}

/// What a parser stopped at, as a syntax error names it (`Code.Next`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Keyword(Keyword),
    Operator(String),
    Close(Bracket),
    /// A capitalized name's first letter and the rest.
    Upper(char, String),
    /// A lower-case name's first letter and, as upstream has it, the
    /// whole name again: reports that put the two together count the
    /// first letter twice.
    Lower(char, String),
    Other(Option<char>),
}

/// One of `Parse.Variable`'s reserved words, the same in every release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    As,
    If,
    In,
    Of,
    Let,
    Case,
    Else,
    Port,
    Then,
    Type,
    Where,
    Import,
    Module,
    Exposing,
}

impl Keyword {
    /// Every reserved word.
    pub const ALL: [Keyword; 14] = [
        Keyword::As,
        Keyword::If,
        Keyword::In,
        Keyword::Of,
        Keyword::Let,
        Keyword::Case,
        Keyword::Else,
        Keyword::Port,
        Keyword::Then,
        Keyword::Type,
        Keyword::Where,
        Keyword::Import,
        Keyword::Module,
        Keyword::Exposing,
    ];

    /// The reserved word `name` is, if it is one.
    pub fn from_name(name: &str) -> Option<Keyword> {
        Keyword::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// The word as written.
    pub const fn as_str(self) -> &'static str {
        match self {
            Keyword::As => "as",
            Keyword::If => "if",
            Keyword::In => "in",
            Keyword::Of => "of",
            Keyword::Let => "let",
            Keyword::Case => "case",
            Keyword::Else => "else",
            Keyword::Port => "port",
            Keyword::Then => "then",
            Keyword::Type => "type",
            Keyword::Where => "where",
            Keyword::Import => "import",
            Keyword::Module => "module",
            Keyword::Exposing => "exposing",
        }
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A kind of closing bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bracket {
    Paren,
    Square,
    Curly,
}

impl Bracket {
    /// What reports call it.
    pub const fn name(self) -> &'static str {
        match self {
            Bracket::Paren => "parenthesis",
            Bracket::Square => "square bracket",
            Bracket::Curly => "curly brace",
        }
    }

    /// The closing bracket itself.
    pub const fn char(self) -> char {
        match self {
            Bracket::Paren => ')',
            Bracket::Square => ']',
            Bracket::Curly => '}',
        }
    }
}

/// `Parse.Symbol.binopCharSet`: the characters operators are made of.
pub const BINOP_CHARS: &str = "+-/*=.<>:&|^?%!";

impl<'a> Source<'a> {
    /// The lines of `text`, as upstream shows them.
    pub fn new(text: &'a str) -> Source<'a> {
        let mut lines: Vec<&str> = text.split_terminator('\n').collect();
        lines.push("");
        Source {
            text,
            index: LineIndex::new(text),
            lines,
        }
    }

    /// Where `range` is, as `compat` keeps positions. Panics if it is
    /// outside the text or splits a character.
    pub fn region(&self, range: TextRange, compat: Compat) -> Region {
        let position = |offset| {
            let elm = self
                .index
                .elm_position(self.text, offset, ColumnUnit::CodePoint)
                .expect("a snippet within its file");
            Position {
                row: compat.keep(elm.row),
                col: compat.keep(elm.column),
            }
        };
        Region {
            start: position(range.start()),
            end: position(range.end()),
        }
    }

    /// `Code.toSnippet` at a region already resolved, as reports about
    /// files other than modules, whose parsers track positions
    /// themselves, show them: `before`, a blank line, the lines of
    /// `region` with `highlight` (or else the whole region) marked, and
    /// `after`.
    pub fn snippet(
        &self,
        compat: Compat,
        before: Doc,
        region: Region,
        highlight: Option<Region>,
        after: Doc,
    ) -> Doc {
        let code = Expander {
            source: self,
            compat,
        }
        .render(region, highlight.unwrap_or(region));
        framed(before, code, after)
    }

    /// The line kept as `row`, if there is one.
    fn line(&self, row: u32, compat: Compat) -> Option<&'a str> {
        let index = row.checked_sub(compat.first_position())?;
        self.lines.get(index as usize).copied()
    }

    /// `Code.whatIsNext`. Every release drops `col - 1` characters of the
    /// line, so from 0.19.2 on, where columns count from 0, it looks one
    /// character early, and at the line's start not at all.
    pub fn what_is_next(&self, at: Position, compat: Compat) -> Next {
        let Some(line) = self.line(at.row, compat) else {
            return Next::Other(None);
        };
        let mut chars = line.chars().skip(at.col.saturating_sub(1) as usize);
        let Some(c) = chars.next() else {
            return Next::Other(None);
        };
        let rest = || {
            chars
                .clone()
                .take_while(|&c| is_inner(c))
                .collect::<String>()
        };
        if ghc::is_upper(c) {
            Next::Upper(c, rest())
        } else if ghc::is_lower(c) {
            let name = format!("{c}{}", rest());
            match Keyword::from_name(&name) {
                Some(keyword) => Next::Keyword(keyword),
                None => Next::Lower(c, name),
            }
        } else if is_binop(c) {
            let mut op = String::from(c);
            op.extend(chars.clone().take_while(|&c| is_binop(c)));
            Next::Operator(op)
        } else {
            match c {
                ')' => Next::Close(Bracket::Paren),
                ']' => Next::Close(Bracket::Square),
                '}' => Next::Close(Bracket::Curly),
                _ => Next::Other(Some(c)),
            }
        }
    }

    /// `Code.nextLineStartsWithKeyword`: where `keyword` is if it starts
    /// the line after `at`'s, after spaces.
    pub fn next_line_starts_with_keyword(
        &self,
        keyword: &str,
        at: Position,
        compat: Compat,
    ) -> Option<Position> {
        self.next_line_starts(at, compat, |rest| {
            rest.strip_prefix(keyword)
                .is_some_and(|after| !after.chars().next().is_some_and(is_inner))
        })
    }

    /// `Code.nextLineStartsWithCloseCurly`.
    pub fn next_line_starts_with_close_curly(
        &self,
        at: Position,
        compat: Compat,
    ) -> Option<Position> {
        self.next_line_starts(at, compat, |rest| rest.starts_with('}'))
    }

    fn next_line_starts(
        &self,
        at: Position,
        compat: Compat,
        starts: impl Fn(&str) -> bool,
    ) -> Option<Position> {
        let row = at.row + 1;
        let line = self.line(row, compat)?;
        let rest = line.trim_start_matches(' ');
        #[expect(clippy::cast_possible_truncation, reason = "a line is shorter")]
        let spaces = (line.len() - rest.len()) as u32;
        starts(rest).then(|| Position {
            row,
            col: compat.wrap(compat.first_position() + spaces),
        })
    }
}

/// `isInner`: a character that continues a name.
fn is_inner(c: char) -> bool {
    ghc::is_alpha_num(c) || c == '_'
}

fn is_binop(c: char) -> bool {
    BINOP_CHARS.contains(c)
}

/// `doc` with its snippets and pairs replaced by the lines of `source`
/// they show, as `compat` shows them.
pub fn expand(doc: &Doc, source: &Source<'_>, compat: Compat) -> Doc {
    Expander { source, compat }.expand(doc)
}

/// What [`expand`] puts snippets in with: a source as a release shows it.
struct Expander<'s, 'a> {
    source: &'s Source<'a>,
    compat: Compat,
}

impl Expander<'_, '_> {
    fn expand(&self, doc: &Doc) -> Doc {
        let boxed = |d: &Doc| Box::new(self.expand(d));
        match doc {
            Doc::Snippet(snippet) => {
                self.render(self.region(snippet.region), self.region(snippet.highlight))
            }
            Doc::Pair(pair) => self.pair(pair),
            Doc::Append(a, b) => Doc::Append(boxed(a), boxed(b)),
            Doc::Nest(width, d) => Doc::Nest(*width, boxed(d)),
            Doc::Align(d) => Doc::Align(boxed(d)),
            Doc::Group(d) => Doc::Group(boxed(d)),
            Doc::Fill(width, d) => Doc::Fill(*width, boxed(d)),
            Doc::Color(color, d) => Doc::Color(*color, boxed(d)),
            Doc::Underline(d) => Doc::Underline(boxed(d)),
            Doc::Empty | Doc::Text(_) | Doc::Line(_) | Doc::Link(_) => doc.clone(),
        }
    }

    fn region(&self, range: TextRange) -> Region {
        self.source.region(range, self.compat)
    }

    /// The row `row` is shown as. Every release numbers lines from 1
    /// (0.19.2's `viewLineNumber` adds the 1 back), except in a pair.
    fn shown(&self, row: u32) -> String {
        (row + 1 - self.compat.first_position()).to_string()
    }

    /// `render`: the lines of `region`, then `highlight` underlined if it
    /// is on the last of them, or else the lines it covers marked.
    fn render(&self, region: Region, highlight: Region) -> Doc {
        let first = self.compat.first_position();
        let all = &self.source.lines;
        let skip = (region.start.row.saturating_sub(first) as usize).min(all.len());
        let take = (region.end.row.saturating_sub(region.start.row) + 1) as usize;
        let lines = &all[skip..all.len().min(skip + take)];
        let top = skip as u32 + first;
        // Upstream fails when there are no lines to take the width from.
        let width = match lines.len() {
            0 => 0,
            n => self.shown(top + n as u32 - 1).len(),
        };
        let underline = self.underline(width, region.end.row, highlight);
        self.draw(width, highlight, top, lines, underline)
    }

    /// `makeUnderline`: carets under a one-row `highlight` on the last
    /// row shown.
    fn underline(&self, width: usize, last_row: u32, highlight: Region) -> Option<Doc> {
        let Region { start, end } = highlight;
        if start.row != end.row || end.row < last_row {
            return None;
        }
        let spaces = start.col as usize + width + 2 - self.compat.first_position() as usize;
        let carets = end.col.saturating_sub(start.col).max(1) as usize;
        Some(text(" ".repeat(spaces)).append(red("^".repeat(carets))))
    }

    /// `drawLines`: each line, the first kept as row `top`, numbered,
    /// then `underline` if there is one, or else a `>` on the lines
    /// `highlight` covers.
    fn draw(
        &self,
        width: usize,
        highlight: Region,
        top: u32,
        lines: &[&str],
        underline: Option<Doc>,
    ) -> Doc {
        let marked = underline.is_none();
        let line = |(&line, row): (&&str, u32)| {
            let spacer = if marked && highlight.start.row <= row && row <= highlight.end.row {
                red(">")
            } else {
                text(" ")
            };
            text(format!("{:>width$}|", self.shown(row)))
                .append(spacer)
                .append(text(line))
        };
        vcat(
            (lines.iter().zip(top..))
                .map(line)
                .chain([underline.unwrap_or_else(Doc::empty)]),
        )
    }

    /// `toPair`.
    fn pair(&self, pair: &Pair) -> Doc {
        let (one, two) = (self.region(pair.first), self.region(pair.second));
        let row = one.start.row;
        if [one.end.row, two.start.row, two.end.row] != [row; 3] {
            let TwoSnippets {
                before,
                between,
                after,
            } = pair.two_snippets.clone();
            return framed(
                before,
                self.render(one, one),
                framed(between, self.render(two, two), after),
            );
        }
        // The row as kept, so from 0.19.2 on one less than elsewhere.
        let number = row.to_string();
        // Upstream fails on a row it doesn't have.
        let line = self.source.line(row, self.compat).unwrap_or_default();
        let first = self.compat.first_position();
        let spaces1 = one.start.col as usize + number.len() + 2 - first as usize;
        let carets =
            |region: Region| "^".repeat(region.end.col.saturating_sub(region.start.col) as usize);
        // Upstream subtracts columns as it keeps them: 0.19.1's 16 bits
        // wrap when the second region starts before the first ends, as a
        // duplicate's do there, into 65,000-odd spaces. Later releases'
        // 32 bits would ask for billions, which none reaches.
        let spaces2 = match self.compat {
            Compat::V0_19_1 => (two.start.col as u16).wrapping_sub(one.end.col as u16) as usize,
            Compat::V0_19_2 | Compat::V0_19_3 => two.start.col.saturating_sub(one.end.col) as usize,
        };
        let code = vcat([
            text(format!("{number}| {line}")),
            text(" ".repeat(spaces1))
                .append(red(carets(one)))
                .append(text(" ".repeat(spaces2)))
                .append(red(carets(two))),
        ]);
        let OneLine { before, after } = pair.one_line.clone();
        framed(before, code, after)
    }
}

#[cfg(test)]
mod tests;
