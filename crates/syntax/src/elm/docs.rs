//! The module documentation's `@docs` lines (`Elm/Docs.hs`,
//! `parseOverview`): every name each one lists, in the order written.
//! The overview is parsed where it sits in the module, so positions are
//! the module's.

use nomi_base::{Compat, TextRange, TextSize};

use super::engine::{P, check_indent, chomp, one_of, one_of_with_fallback, word1};
use super::error::{BadOperator, Cursor, Space};
use super::literal::{inner_width, lower, operator, upper};

/// What a name in an `@docs` line is (0.19.3's `DocName`; earlier
/// releases keep the three in one namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocKind {
    Type,
    Value,
    Binop,
}

/// A name an `@docs` line lists, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocName {
    pub kind: DocKind,
    pub name: String,
    pub range: TextRange,
}

/// `Reporting.Error.Docs.SyntaxProblem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxProblem {
    Op(Cursor),
    OpBad(BadOperator, Cursor),
    Name(Cursor),
    Space(Space, Cursor),
    Comma(Cursor),
    BadEnd(Cursor),
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
fn cursor_at(p: &P<'_>, src: &str, offset: usize) -> Cursor {
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
    loop {
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
        let more = one_of_with_fallback(
            p,
            &mut [&mut |p| {
                let pos = p.cur;
                check_indent(p, pos, SyntaxProblem::Comma)?;
                word1(p, b',', SyntaxProblem::Comma)?;
                chomp(p, SyntaxProblem::Space)?;
                Ok(true)
            }],
            false,
        )?;
        if !more {
            return Ok(());
        }
    }
}

fn size(offset: usize) -> TextSize {
    TextSize::try_from(offset).expect("a module under 4 GiB")
}

/// `chompOperator`: an operator in parentheses.
fn chomp_operator(p: &mut P<'_>) -> Result<Box<str>, SyntaxProblem> {
    word1(p, b'(', SyntaxProblem::Op)?;
    let op = operator(p, SyntaxProblem::Op, SyntaxProblem::OpBad)?;
    word1(p, b')', SyntaxProblem::Op)?;
    Ok(op)
}

/// `chompUntilDocs`: on to just past the next `@docs` not followed by
/// more of a name, and whether there was one. 0.19.1 takes one ending
/// the comment; later releases want a character after it.
fn until_docs(p: &mut P<'_>) -> bool {
    let (mut pos, mut cur) = (p.pos, p.cur);
    let end = p.len();
    let found = loop {
        let Some(byte) = p.byte(pos) else {
            break false;
        };
        if byte == b'\n' {
            pos += 1;
            cur = p.newline(cur);
            continue;
        }
        let pos5 = pos + 5;
        let room = if p.v1() { pos5 <= end } else { pos5 < end };
        if room && p.bytes(pos, pos5) == b"@docs" && inner_width(p, pos5) == 0 {
            pos = pos5;
            cur = p.slide(cur, 5);
            break true;
        }
        pos += char_width(byte);
        cur = p.slide(cur, 1);
    };
    p.pos = pos;
    p.cur = cur;
    found
}

/// `getCharWidth`, of a byte that starts a character.
fn char_width(byte: u8) -> usize {
    match byte {
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        0xF0.. => 4,
        _ => 1,
    }
}
