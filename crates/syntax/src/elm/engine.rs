//! `Parse/Primitives.hs` and `Parse/Space.hs`.
//!
//! Upstream's parsers take four continuations: consumed or empty, ok or
//! error. Here a parser is a function of the state returning a `Result`,
//! and whether it consumed is a flag on the state that every primitive
//! which advances sets and nothing clears but [`one_of`], which runs each
//! alternative from a clear flag to see whether it may try the next. A
//! primitive that fails without consuming leaves the state as it found
//! it; a parser that fails after an empty success may not, so whatever
//! catches the failure puts back the state it saved.
//!
//! Upstream loops by recursion; here a loop's step says whether to go on
//! with a [`ControlFlow`], and [`repeat`], [`fold_more`] and
//! [`comma_until`] run the three loops upstream writes over and over.

use std::ops::ControlFlow;

use nomi_base::{Compat, utf8};

use super::error::{Position, Space};

/// `State`, and whether the parser running has consumed.
pub(super) struct P<'s> {
    src: &'s [u8],
    pub(super) pos: usize,
    indent: u32,
    pub(super) cur: Position,
    consumed: bool,
    pub(super) compat: Compat,
}

/// What [`P::restore`] puts back.
#[derive(Clone, Copy)]
struct Saved {
    pos: usize,
    indent: u32,
    cur: Position,
    consumed: bool,
}

/// A place in the module: its byte offset, and the row and column there.
#[derive(Clone, Copy)]
pub(super) struct Loc {
    pub(super) pos: usize,
    pub(super) cur: Position,
}

impl Loc {
    /// `bytes` on and `cols` columns right, in the same row.
    pub(super) fn step(self, p: &P<'_>, bytes: usize, cols: u32) -> Loc {
        Loc {
            pos: self.pos + bytes,
            cur: p.slide(self.cur, cols),
        }
    }

    /// Past the newline here, at the start of the next row.
    pub(super) fn next_line(self, p: &P<'_>) -> Loc {
        Loc {
            pos: self.pos + 1,
            cur: p.newline(self.cur),
        }
    }
}

/// An alternative for [`one_of`].
type Alt<'a, 's, T, X> = &'a mut dyn FnMut(&mut P<'s>) -> Result<T, X>;

impl<'s> P<'s> {
    pub(super) fn new(src: &'s [u8], compat: Compat) -> P<'s> {
        let first = compat.first_position();
        P {
            src,
            pos: 0,
            indent: 0,
            cur: Position {
                row: first,
                col: first,
            },
            consumed: false,
            compat,
        }
    }

    pub(super) fn v1(&self) -> bool {
        self.compat == Compat::V0_19_1
    }

    /// The column a line starts at.
    fn first(&self) -> u32 {
        self.compat.first_position()
    }

    /// A row or column as a `Word`: 16 bits on 0.19.1, 32 later.
    pub(super) fn wrap(&self, n: u32) -> u32 {
        self.compat.wrap(n)
    }

    pub(super) fn src(&self) -> &'s [u8] {
        self.src
    }

    pub(super) fn len(&self) -> usize {
        self.src.len()
    }

    /// Where the parser stands.
    pub(super) fn here(&self) -> Loc {
        Loc {
            pos: self.pos,
            cur: self.cur,
        }
    }

    /// The byte `off` past the position, if there is one.
    pub(super) fn peek(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    pub(super) fn byte(&self, pos: usize) -> Option<u8> {
        self.src.get(pos).copied()
    }

    pub(super) fn bytes(&self, from: usize, to: usize) -> &'s [u8] {
        &self.src[from..to.min(self.src.len())]
    }

    pub(super) fn is_word(&self, pos: usize, word: u8) -> bool {
        self.byte(pos) == Some(word)
    }

    /// `slide`: `n` columns on.
    pub(super) fn slide(&self, cur: Position, n: u32) -> Position {
        Position {
            row: cur.row,
            col: self.wrap(cur.col.wrapping_add(n)),
        }
    }

    /// `newline`: the start of the next row.
    pub(super) fn newline(&self, cur: Position) -> Position {
        Position {
            row: self.wrap(cur.row.wrapping_add(1)),
            col: self.first(),
        }
    }

    fn is_indented(&self, cur: Position) -> bool {
        cur.col > self.indent && cur.col > self.first()
    }

    fn save(&self) -> Saved {
        Saved {
            pos: self.pos,
            indent: self.indent,
            cur: self.cur,
            consumed: self.consumed,
        }
    }

    fn restore(&mut self, saved: Saved) {
        self.pos = saved.pos;
        self.indent = saved.indent;
        self.cur = saved.cur;
        self.consumed = saved.consumed;
    }

    /// A consumed success, at `at`.
    pub(super) fn ok_at(&mut self, at: Loc) {
        self.pos = at.pos;
        self.cur = at.cur;
        self.consumed = true;
    }

    /// A consumed success, `bytes` on and `cols` columns right.
    pub(super) fn advance(&mut self, bytes: usize, cols: u32) {
        self.ok_at(self.here().step(self, bytes, cols));
    }

    /// A consumed failure.
    pub(super) fn cerr<T, X>(&mut self, x: X) -> Result<T, X> {
        self.consumed = true;
        Err(x)
    }

    /// The width of the character at `pos` as `skipUtf8` finds it: none
    /// for a zero byte or bytes that aren't UTF-8. 0.19.1 trusts the
    /// first byte, and dies on one that can't start a character, which
    /// counts as one byte here.
    pub(super) fn skip_utf8(&self, pos: usize) -> usize {
        let Some(&b) = self.src.get(pos) else {
            return 0;
        };
        if self.v1() {
            return if b >= 0xF8 { 1 } else { utf8::char_width(b) };
        }
        if b == 0 {
            return 0;
        }
        let width = utf8::char_width(b);
        match self.src.get(pos..pos + width) {
            Some(bytes) if std::str::from_utf8(bytes).is_ok() => width,
            _ => 0,
        }
    }
}

/// `oneOf`: the first alternative that succeeds or fails having consumed,
/// or `to_error` where it started.
pub(super) fn one_of<'s, T, X>(
    p: &mut P<'s>,
    to_error: impl FnOnce(Position) -> X,
    alts: &mut [Alt<'_, 's, T, X>],
) -> Result<T, X> {
    first_of(p, alts).unwrap_or_else(|| Err(to_error(p.cur)))
}

/// `oneOfWithFallback`: [`one_of`], succeeding with `fallback` where it
/// started if none does.
pub(super) fn one_of_with_fallback<'s, T, X>(
    p: &mut P<'s>,
    alts: &mut [Alt<'_, 's, T, X>],
    fallback: T,
) -> Result<T, X> {
    first_of(p, alts).unwrap_or(Ok(fallback))
}

/// The first alternative to succeed or to fail having consumed, if any
/// does; if none does, the state is as it was.
fn first_of<'s, T, X>(p: &mut P<'s>, alts: &mut [Alt<'_, 's, T, X>]) -> Option<Result<T, X>> {
    let saved = p.save();
    for alt in alts {
        p.consumed = false;
        match alt(p) {
            Ok(value) => {
                p.consumed |= saved.consumed;
                return Some(Ok(value));
            }
            Err(e) if p.consumed => return Some(Err(e)),
            Err(_) => p.restore(saved),
        }
    }
    None
}

/// `step` again and again, until it breaks with a value.
pub(super) fn repeat<'s, T, X>(
    p: &mut P<'s>,
    mut step: impl FnMut(&mut P<'s>) -> Result<ControlFlow<T>, X>,
) -> Result<T, X> {
    loop {
        if let ControlFlow::Break(value) = step(p)? {
            return Ok(value);
        }
    }
}

/// `first`, then each value `next` parses from the one before, for as
/// long as it parses, as upstream loops through `oneOfWithFallback`: the
/// last value.
pub(super) fn fold_more<'s, T: Copy, X>(
    p: &mut P<'s>,
    first: T,
    mut next: impl FnMut(&mut P<'s>, T) -> Result<T, X>,
) -> Result<T, X> {
    let mut last = first;
    loop {
        let step = one_of_with_fallback(
            p,
            &mut [&mut |p| next(p, last).map(ControlFlow::Continue)],
            ControlFlow::Break(last),
        )?;
        match step {
            ControlFlow::Continue(value) => last = value,
            ControlFlow::Break(value) => return Ok(value),
        }
    }
}

/// A `,` and `item`, for as long as a comma comes, then `close`: how
/// lists, tuples, records and exposing lists end, with `to_end` where
/// neither comes.
pub(super) fn comma_until<'s, X>(
    p: &mut P<'s>,
    close: u8,
    to_end: fn(Position) -> X,
    mut item: impl FnMut(&mut P<'s>) -> Result<(), X>,
) -> Result<(), X> {
    repeat(p, |p| {
        one_of(
            p,
            to_end,
            &mut [
                &mut |p| {
                    word1(p, b',', to_end)?;
                    item(p).map(ControlFlow::Continue)
                },
                &mut |p| word1(p, close, to_end).map(ControlFlow::Break),
            ],
        )
    })
}

/// `inContext`: `start`, then `parser`, whose errors are wrapped with
/// where `start` began.
pub(super) fn in_context<'s, T, X, Y>(
    p: &mut P<'s>,
    add: impl FnOnce(X, Position) -> Y,
    start: impl FnOnce(&mut P<'s>) -> Result<(), Y>,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, Y> {
    let cur = p.cur;
    start(p)?;
    parser(p).map_err(|e| add(e, cur))
}

/// `specialize`: `parser`, whose errors are wrapped with where it began.
pub(super) fn specialize<'s, T, X, Y>(
    p: &mut P<'s>,
    add: impl FnOnce(X, Position) -> Y,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, Y> {
    let cur = p.cur;
    parser(p).map_err(|e| add(e, cur))
}

/// The error constructor `wrap`, boxing the error it wraps.
pub(super) fn boxed<X, Y>(wrap: fn(Box<X>, Position) -> Y) -> impl FnOnce(X, Position) -> Y {
    move |x, cur| wrap(Box::new(x), cur)
}

/// `withIndent`: `parser` with the indent at the current column.
pub(super) fn with_indent<'s, T, X>(
    p: &mut P<'s>,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, X> {
    with_backset_indent(p, 0, parser)
}

/// `withBacksetIndent`: `parser` with the indent `backset` columns left
/// of the current one, wrapping as upstream's words do.
pub(super) fn with_backset_indent<'s, T, X>(
    p: &mut P<'s>,
    backset: u32,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, X> {
    let old = p.indent;
    p.indent = p.wrap(p.cur.col.wrapping_sub(backset));
    let value = parser(p)?;
    p.indent = old;
    Ok(value)
}

/// `word1`.
pub(super) fn word1<X>(
    p: &mut P<'_>,
    word: u8,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    if p.peek(0) == Some(word) {
        p.advance(1, 1);
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

/// `word2`.
pub(super) fn word2<X>(
    p: &mut P<'_>,
    w1: u8,
    w2: u8,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    if p.peek(0) == Some(w1) && p.peek(1) == Some(w2) {
        p.advance(2, 2);
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

// SPACE

/// `eatSpaces`: the spaces and comments from `at`, and where they end,
/// or what stopped them and where.
fn eat_spaces(p: &P<'_>, mut at: Loc) -> Result<Loc, (Space, Position)> {
    loop {
        let Some(b) = p.byte(at.pos) else {
            return Ok(at);
        };
        match b {
            b' ' => at = at.step(p, 1, 1),
            b'\n' => at = at.next_line(p),
            b'\r' => at.pos += 1,
            b'\t' => return Err((Space::HasTab, at.cur)),
            b'{' => {
                if at.pos + 2 >= p.len()
                    || !p.is_word(at.pos + 1, b'-')
                    || p.is_word(at.pos + 2, b'|')
                {
                    return Ok(at);
                }
                at = eat_multi_comment(p, at, 2)?;
            }
            b'-' if p.is_word(at.pos + 1, b'-') => {
                at = at.step(p, 2, 2);
                loop {
                    match p.byte(at.pos) {
                        None => return Ok(at),
                        Some(b'\n') => {
                            at = at.next_line(p);
                            break;
                        }
                        Some(_) => match p.skip_utf8(at.pos) {
                            0 => return Err((Space::HasNonUtf8, at.cur)),
                            width => at = at.step(p, width, 1),
                        },
                    }
                }
            }
            _ => return Ok(at),
        }
    }
}

/// `eatMultiCommentHelp`, from the `{-` (or `{-|`, `open` bytes long) at
/// `start`: where the comment ends, or what stopped it and where, an
/// endless one where it started.
fn eat_multi_comment(p: &P<'_>, start: Loc, open: usize) -> Result<Loc, (Space, Position)> {
    let mut at = start.step(p, open, open as u32);
    let mut depth = 1u32;
    loop {
        let Some(b) = p.byte(at.pos) else {
            return Err((Space::EndlessMultiComment, start.cur));
        };
        match b {
            b'\n' => at = at.next_line(p),
            b'\t' => return Err((Space::HasTab, at.cur)),
            b'-' if p.is_word(at.pos + 1, b'}') => {
                at = at.step(p, 2, 2);
                if depth == 1 {
                    return Ok(at);
                }
                depth -= 1;
            }
            b'{' if p.is_word(at.pos + 1, b'-') => {
                at = at.step(p, 2, 2);
                depth += 1;
            }
            _ => match p.skip_utf8(at.pos) {
                0 => return Err((Space::HasNonUtf8, at.cur)),
                width => at = at.step(p, width, 1),
            },
        }
    }
}

/// `Space.chomp`.
pub(super) fn chomp<X>(
    p: &mut P<'_>,
    to_error: impl FnOnce(Space, Position) -> X,
) -> Result<(), X> {
    match eat_spaces(p, p.here()) {
        Ok(at) => {
            p.ok_at(at);
            Ok(())
        }
        Err((space, cur)) => p.cerr(to_error(space, cur)),
    }
}

/// `chompAndCheckIndent`.
pub(super) fn chomp_and_check_indent<X>(
    p: &mut P<'_>,
    to_space_error: impl FnOnce(Space, Position) -> X,
    to_indent_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    match eat_spaces(p, p.here()) {
        Ok(at) if p.is_indented(at.cur) => {
            p.ok_at(at);
            Ok(())
        }
        Ok(_) => {
            let x = to_indent_error(p.cur);
            p.cerr(x)
        }
        Err((space, cur)) => p.cerr(to_space_error(space, cur)),
    }
}

/// `checkIndent`: whether the spaces just chomped end indented, or
/// `to_error` where they began.
pub(super) fn check_indent<X>(
    p: &P<'_>,
    end: Position,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    if p.is_indented(p.cur) {
        Ok(())
    } else {
        Err(to_error(end))
    }
}

/// `checkAligned`.
pub(super) fn check_aligned<X>(
    p: &P<'_>,
    to_error: impl FnOnce(u32, Position) -> X,
) -> Result<(), X> {
    if p.cur.col == p.indent {
        Ok(())
    } else {
        Err(to_error(p.indent, p.cur))
    }
}

/// `checkFreshLine`.
pub(super) fn check_fresh_line<X>(
    p: &P<'_>,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    if p.cur.col == p.first() {
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

/// `docComment`.
pub(super) fn doc_comment<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Position) -> X,
    to_space_error: impl FnOnce(Space, Position) -> X,
) -> Result<(), X> {
    if !(p.peek(0) == Some(b'{') && p.peek(1) == Some(b'-') && p.peek(2) == Some(b'|')) {
        return Err(to_expectation(p.cur));
    }
    match eat_multi_comment(p, p.here(), 3) {
        Ok(at) => {
            p.ok_at(at);
            Ok(())
        }
        Err((space, cur)) => p.cerr(to_space_error(space, cur)),
    }
}
