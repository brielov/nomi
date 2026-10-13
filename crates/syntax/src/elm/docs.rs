//! The module documentation's `@docs` lines (`Elm/Docs.hs`,
//! `parseOverview`): every name each one lists, in the order written.
//! The overview is parsed where it sits in the module, so positions are
//! the module's.

use std::ops::ControlFlow;

use nomi_base::{Compat, TextRange, TextSize, utf8};

use super::engine::{P, check_indent, chomp, one_of, one_of_with_fallback, repeat, word1};
use super::error::{Position, Space};
use super::literal::{inner_width, lower, operator, upper};

/// What a name in an `@docs` line is (0.19.3's `DocName`; earlier
/// releases keep the three in one namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    /// A capitalized name: a type or type alias.
    Type,
    /// A lower-case name: a value.
    Value,
    /// An operator in parentheses.
    Binop,
}

/// A name an `@docs` line lists, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocName {
    pub kind: DocKind,
    /// The name, without an operator's parentheses.
    pub name: String,
    /// Where it is in the module, with an operator's parentheses.
    pub range: TextRange,
}

/// `Reporting.Error.Docs.SyntaxProblem`: what stops an `@docs` line, and
/// where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxProblem {
    /// Something other than an operator in parentheses.
    Op(Position),
    /// A reserved symbol in parentheses.
    OpBad(Position),
    /// Something other than a name.
    Name(Position),
    /// A tab, bytes that aren't UTF-8, or an endless comment.
    Space(Space, Position),
    /// Something other than a comma after a name.
    Comma(Position),
    /// What is left over after the `@docs` lines.
    BadEnd(Position),
}

/// `parseOverview`: the names the `@docs` lines of the doc comment whose
/// text, between `{-|` and `-}`, is `src[start..end]` list, in the
/// order written.
pub fn overview(
    src: &str,
    start: usize,
    end: usize,
    compat: Compat,
) -> Result<Vec<DocName>, SyntaxProblem> {
    let mut p = P::new(&src.as_bytes()[..end], compat);
    p.cur = cursor_at(&p, src, start);
    p.pos = start;
    let mut names = Vec::new();
    while until_docs(&mut p) {
        chomp(&mut p, SyntaxProblem::Space)?;
        docs(&mut p, &mut names)?;
    }
    // `fromSnippet`: what is left over is `BadEnd`, which `untilDocs`,
    // running to the end, never leaves.
    if p.pos != p.len() {
        return Err(SyntaxProblem::BadEnd(p.cur));
    }
    Ok(names)
}

/// Where the parser stands at `offset`: past the module's lines, and the
/// characters of the last one, but carriage returns, which `eatSpaces`
/// skips without a column.
fn cursor_at(p: &P<'_>, src: &str, offset: usize) -> Position {
    let mut cur = p.cur;
    for c in src[..offset].chars() {
        match c {
            '\n' => cur = p.newline(cur),
            '\r' => {}
            _ => cur = p.slide(cur, 1),
        }
    }
    cur
}

/// `chompDocs`: one name, then more after commas.
fn docs(p: &mut P<'_>, names: &mut Vec<DocName>) -> Result<(), SyntaxProblem> {
    repeat(p, |p| {
        let start = p.pos;
        let (kind, name) = one_of(
            p,
            SyntaxProblem::Name,
            &mut [
                &mut |p| lower(p, SyntaxProblem::Name).map(|n| (DocKind::Value, n)),
                &mut |p| upper(p, SyntaxProblem::Name).map(|n| (DocKind::Type, n)),
                &mut |p| chomp_operator(p).map(|n| (DocKind::Binop, n)),
            ],
        )?;
        names.push(DocName {
            kind,
            name: name.into(),
            range: TextRange::new(size(start), size(p.pos)),
        });
        chomp(p, SyntaxProblem::Space)?;
        one_of_with_fallback(
            p,
            &mut [&mut |p| {
                let pos = p.cur;
                check_indent(p, pos, SyntaxProblem::Comma)?;
                word1(p, b',', SyntaxProblem::Comma)?;
                chomp(p, SyntaxProblem::Space)?;
                Ok(ControlFlow::Continue(()))
            }],
            ControlFlow::Break(()),
        )
    })
}

fn size(offset: usize) -> TextSize {
    TextSize::try_from(offset).expect("a module under 4 GiB")
}

/// `chompOperator`: an operator in parentheses.
fn chomp_operator(p: &mut P<'_>) -> Result<Box<str>, SyntaxProblem> {
    word1(p, b'(', SyntaxProblem::Op)?;
    let op = operator(p, SyntaxProblem::Op, |_, at| SyntaxProblem::OpBad(at))?;
    word1(p, b')', SyntaxProblem::Op)?;
    Ok(op)
}

/// `chompUntilDocs`: on to just past the next `@docs` not followed by
/// more of a name, and whether there was one. 0.19.1 takes one ending
/// the comment; later releases want a character after it.
fn until_docs(p: &mut P<'_>) -> bool {
    let mut at = p.here();
    let end = p.len();
    let found = loop {
        let Some(byte) = p.byte(at.pos) else {
            break false;
        };
        if byte == b'\n' {
            at = at.next_line(p);
            continue;
        }
        let pos5 = at.pos + 5;
        let room = if p.v1() { pos5 <= end } else { pos5 < end };
        if room && p.bytes(at.pos, pos5) == b"@docs" && inner_width(p, pos5) == 0 {
            at = at.step(p, 5, 5);
            break true;
        }
        at = at.step(p, utf8::char_width(byte), 1);
    };
    p.pos = at.pos;
    p.cur = at.cur;
    found
}
