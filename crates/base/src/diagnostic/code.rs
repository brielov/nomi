//! Source snippets: a port of `Reporting/Render/Code.hs`. Documents name
//! the code they show by byte range, so a diagnostic doesn't hold its
//! file's text; [`expand`] puts the lines in, numbered and marked as the
//! release does, before a document is laid out.
//!
//! Releases keep positions differently, and it shows: 0.19.1 counts rows
//! and columns from 1 in 16 bits, later releases from 0. So a pair on one
//! row is numbered one less from 0.19.2 on, while every other snippet is
//! numbered from 1.

use text_size::TextRange;

use super::doc::{Color, Doc, Pair, text, vcat};
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

impl<'a> Source<'a> {
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
            let keep = |from_1: u32| {
                let kept = from_1 - 1 + compat.first_position();
                compat.column_wrap().map_or(kept, |wrap| kept % wrap)
            };
            Position {
                row: keep(elm.row),
                col: keep(elm.column),
            }
        };
        Region {
            start: position(range.start()),
            end: position(range.end()),
        }
    }
}

impl Source<'_> {
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
        let code = Code {
            source: self,
            compat,
        }
        .render(region, highlight.unwrap_or(region));
        vcat([before, Doc::empty(), code, after])
    }
}

/// What a parser stopped at, as a syntax error names it (`Code.Next`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    Keyword(&'static str),
    Operator(String),
    /// What the bracket is called, and the bracket.
    Close(&'static str, char),
    /// A capitalized name's first letter and the rest.
    Upper(char, String),
    /// A lower-case name's first letter and, as upstream has it, the
    /// whole name again: reports that put the two together count the
    /// first letter twice.
    Lower(char, String),
    Other(Option<char>),
}

/// `Parse.Variable`'s reserved words, the same in every release.
const KEYWORDS: [&str; 14] = [
    "as", "if", "in", "of", "let", "case", "else", "port", "then", "type", "where", "import",
    "module", "exposing",
];

impl Source<'_> {
    /// The line kept as `row`, if there is one.
    fn line(&self, row: u32, compat: Compat) -> Option<&str> {
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
            match KEYWORDS.iter().find(|&&k| k == name) {
                Some(keyword) => Next::Keyword(keyword),
                None => Next::Lower(c, name),
            }
        } else if is_binop(c) {
            let mut op = String::from(c);
            op.extend(chars.clone().take_while(|&c| is_binop(c)));
            Next::Operator(op)
        } else {
            match c {
                ')' => Next::Close("parenthesis", ')'),
                ']' => Next::Close("square bracket", ']'),
                '}' => Next::Close("curly brace", '}'),
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
        #[allow(clippy::cast_possible_truncation, reason = "a line is shorter")]
        let spaces = (line.len() - rest.len()) as u32;
        let col = compat.first_position() + spaces;
        starts(rest).then(|| Position {
            row,
            col: compat.column_wrap().map_or(col, |wrap| col % wrap),
        })
    }
}

/// `isInner`: a character that continues a name.
fn is_inner(c: char) -> bool {
    ghc::is_alpha_num(c) || c == '_'
}

/// `Parse.Symbol.binopCharSet`.
fn is_binop(c: char) -> bool {
    "+-/*=.<>:&|^?%!".contains(c)
}

/// `doc` with its snippets and pairs replaced by the lines of `source`
/// they show, as `compat` shows them.
pub fn expand(doc: &Doc, source: &Source<'_>, compat: Compat) -> Doc {
    Code { source, compat }.expand(doc)
}

struct Code<'s, 'a> {
    source: &'s Source<'a>,
    compat: Compat,
}

impl Code<'_, '_> {
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
            Doc::Bold(d) => Doc::Bold(boxed(d)),
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
        let lines: Vec<(u32, &str)> = (self.source.lines.iter().enumerate())
            .skip(region.start.row.saturating_sub(first) as usize)
            .take((region.end.row.saturating_sub(region.start.row) + 1) as usize)
            .map(|(i, line)| (i as u32 + first, *line))
            .collect();
        // Upstream fails when there are no lines to take the width from.
        let width = lines.last().map_or(0, |&(row, _)| self.shown(row).len());
        match self.underline(width, region.end.row, highlight) {
            Some(underline) => self.draw(false, width, highlight, &lines, underline),
            None => self.draw(true, width, highlight, &lines, Doc::empty()),
        }
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

    /// `drawLines`: each line numbered, with a `>` on those `highlight`
    /// covers if `marked`, then `last`.
    fn draw(
        &self,
        marked: bool,
        width: usize,
        highlight: Region,
        lines: &[(u32, &str)],
        last: Doc,
    ) -> Doc {
        let line = |&(row, line): &(u32, &str)| {
            let spacer = if marked && highlight.start.row <= row && row <= highlight.end.row {
                red(">")
            } else {
                text(" ")
            };
            text(format!("{:>width$}|", self.shown(row)))
                .append(spacer)
                .append(text(line))
        };
        vcat(lines.iter().map(line).chain([last]))
    }

    /// `toPair`.
    fn pair(&self, pair: &Pair) -> Doc {
        let (one, two) = (self.region(pair.first), self.region(pair.second));
        let row = one.start.row;
        if [one.end.row, two.start.row, two.end.row] != [row; 3] {
            let (before, between, after) = pair.two_chunks.clone();
            return vcat([
                before,
                Doc::empty(),
                self.render(one, one),
                between,
                Doc::empty(),
                self.render(two, two),
                after,
            ]);
        }
        // The row as kept, so from 0.19.2 on one less than elsewhere.
        let number = row.to_string();
        let first = self.compat.first_position();
        // Upstream fails on a row it doesn't have.
        let line = (row.checked_sub(first))
            .and_then(|i| self.source.lines.get(i as usize))
            .copied()
            .unwrap_or_default();
        let spaces1 = one.start.col as usize + number.len() + 2 - first as usize;
        let carets =
            |region: Region| "^".repeat(region.end.col.saturating_sub(region.start.col) as usize);
        let spaces2 = two.start.col.saturating_sub(one.end.col) as usize;
        let code = vcat([
            text(format!("{number}| {line}")),
            text(" ".repeat(spaces1))
                .append(red(carets(one)))
                .append(text(" ".repeat(spaces2)))
                .append(red(carets(two))),
        ]);
        let (before, after) = pair.one_line.clone();
        vcat([before, Doc::empty(), code, after])
    }
}

/// `D.red`, which styles even no text.
fn red(chars: impl Into<String>) -> Doc {
    text(chars).color(Color::Red)
}

#[cfg(test)]
mod tests;
