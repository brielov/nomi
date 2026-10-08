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

use nomi_base::Compat;

use super::error::{Cursor, Space};

/// `State`, and whether the parser running has consumed.
pub(super) struct P<'s> {
    src: &'s [u8],
    pub(super) pos: usize,
    pub(super) indent: u32,
    pub(super) cur: Cursor,
    pub(super) consumed: bool,
    pub(super) compat: Compat,
}

#[derive(Clone, Copy)]
pub(super) struct Saved {
    pos: usize,
    indent: u32,
    cur: Cursor,
    consumed: bool,
}

/// An alternative for [`one_of`].
pub(super) type Alt<'a, 's, T, X> = &'a mut dyn FnMut(&mut P<'s>) -> Result<T, X>;

impl<'s> P<'s> {
    pub(super) fn new(src: &'s [u8], compat: Compat) -> P<'s> {
        let first = if compat == Compat::V0_19_1 { 1 } else { 0 };
        P {
            src,
            pos: 0,
            indent: 0,
            cur: Cursor {
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
    pub(super) fn first(&self) -> u32 {
        u32::from(self.v1())
    }

    /// A 16-bit `Word` on 0.19.1, a 32-bit one later.
    pub(super) fn word(&self, n: u32) -> u32 {
        if self.v1() { n & 0xFFFF } else { n }
    }

    pub(super) fn len(&self) -> usize {
        self.src.len()
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
    pub(super) fn slide(&self, cur: Cursor, n: u32) -> Cursor {
        Cursor {
            row: cur.row,
            col: self.word(cur.col.wrapping_add(n)),
        }
    }

    /// `newline`: the start of the next row.
    pub(super) fn newline(&self, cur: Cursor) -> Cursor {
        Cursor {
            row: self.word(cur.row.wrapping_add(1)),
            col: self.first(),
        }
    }

    pub(super) fn is_indented(&self, cur: Cursor) -> bool {
        cur.col > self.indent && cur.col > self.first()
    }

    pub(super) fn save(&self) -> Saved {
        Saved {
            pos: self.pos,
            indent: self.indent,
            cur: self.cur,
            consumed: self.consumed,
        }
    }

    pub(super) fn restore(&mut self, saved: Saved) {
        self.pos = saved.pos;
        self.indent = saved.indent;
        self.cur = saved.cur;
        self.consumed = saved.consumed;
    }

    /// A consumed success: `bytes` on, at `cur`.
    pub(super) fn ok_at(&mut self, pos: usize, cur: Cursor) {
        self.pos = pos;
        self.cur = cur;
        self.consumed = true;
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
            return match b {
                0x00..0x80 | 0x80..0xC0 | 0xF8.. => 1,
                0xC0..0xE0 => 2,
                0xE0..0xF0 => 3,
                0xF0..0xF8 => 4,
            };
        }
        if b == 0 {
            return 0;
        }
        let width = match b {
            0x01..0x80 => return 1,
            0xC0..0xE0 => 2,
            0xE0..0xF0 => 3,
            _ => 4,
        };
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
    to_error: impl FnOnce(Cursor) -> X,
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

/// `inContext`: `start`, then `parser`, whose errors are wrapped with
/// where `start` began.
pub(super) fn in_context<'s, S, T, X, Y>(
    p: &mut P<'s>,
    add: impl FnOnce(X, Cursor) -> Y,
    start: impl FnOnce(&mut P<'s>) -> Result<S, Y>,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, Y> {
    let cur = p.cur;
    start(p)?;
    parser(p).map_err(|e| add(e, cur))
}

/// `specialize`: `parser`, whose errors are wrapped with where it began.
pub(super) fn specialize<'s, T, X, Y>(
    p: &mut P<'s>,
    add: impl FnOnce(X, Cursor) -> Y,
    parser: impl FnOnce(&mut P<'s>) -> Result<T, X>,
) -> Result<T, Y> {
    let cur = p.cur;
    parser(p).map_err(|e| add(e, cur))
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
    p.indent = p.word(p.cur.col.wrapping_sub(backset));
    let value = parser(p)?;
    p.indent = old;
    Ok(value)
}

/// `word1`.
pub(super) fn word1<X>(
    p: &mut P<'_>,
    word: u8,
    to_error: impl FnOnce(Cursor) -> X,
) -> Result<(), X> {
    if p.peek(0) == Some(word) {
        p.ok_at(p.pos + 1, p.slide(p.cur, 1));
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
    to_error: impl FnOnce(Cursor) -> X,
) -> Result<(), X> {
    if p.peek(0) == Some(w1) && p.peek(1) == Some(w2) {
        p.ok_at(p.pos + 2, p.slide(p.cur, 2));
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

// SPACE

enum Multi {
    Good,
    Tab,
    NonUtf8,
    Endless,
}

/// `eatSpaces`: the spaces and comments from `pos`, and where they end.
fn eat_spaces(p: &P<'_>, mut pos: usize, mut cur: Cursor) -> (Option<Space>, usize, Cursor) {
    loop {
        let Some(b) = p.byte(pos) else {
            return (None, pos, cur);
        };
        match b {
            b' ' => {
                pos += 1;
                cur = p.slide(cur, 1);
            }
            b'\n' => {
                pos += 1;
                cur = p.newline(cur);
            }
            b'\r' => pos += 1,
            b'\t' => return (Some(Space::HasTab), pos, cur),
            b'{' => {
                if pos + 2 >= p.len() || !p.is_word(pos + 1, b'-') || p.is_word(pos + 2, b'|') {
                    return (None, pos, cur);
                }
                let (status, new_pos, new_cur) = eat_multi_comment(p, pos + 2, p.slide(cur, 2));
                match status {
                    Multi::Good => (pos, cur) = (new_pos, new_cur),
                    Multi::Tab => return (Some(Space::HasTab), new_pos, new_cur),
                    Multi::NonUtf8 => return (Some(Space::HasNonUtf8), new_pos, new_cur),
                    Multi::Endless => return (Some(Space::EndlessMultiComment), pos, cur),
                }
            }
            b'-' if p.is_word(pos + 1, b'-') => {
                pos += 2;
                cur = p.slide(cur, 2);
                loop {
                    match p.byte(pos) {
                        None => return (None, pos, cur),
                        Some(b'\n') => {
                            pos += 1;
                            cur = p.newline(cur);
                            break;
                        }
                        Some(_) => {
                            let width = p.skip_utf8(pos);
                            if width == 0 {
                                return (Some(Space::HasNonUtf8), pos, cur);
                            }
                            pos += width;
                            cur = p.slide(cur, 1);
                        }
                    }
                }
            }
            _ => return (None, pos, cur),
        }
    }
}

/// `eatMultiCommentHelp`, from inside the first `{-`.
fn eat_multi_comment(p: &P<'_>, mut pos: usize, mut cur: Cursor) -> (Multi, usize, Cursor) {
    let mut open = 1u32;
    loop {
        let Some(b) = p.byte(pos) else {
            return (Multi::Endless, pos, cur);
        };
        match b {
            b'\n' => {
                pos += 1;
                cur = p.newline(cur);
            }
            b'\t' => return (Multi::Tab, pos, cur),
            b'-' if p.is_word(pos + 1, b'}') => {
                pos += 2;
                cur = p.slide(cur, 2);
                if open == 1 {
                    return (Multi::Good, pos, cur);
                }
                open -= 1;
            }
            b'{' if p.is_word(pos + 1, b'-') => {
                pos += 2;
                cur = p.slide(cur, 2);
                open += 1;
            }
            _ => {
                let width = p.skip_utf8(pos);
                if width == 0 {
                    return (Multi::NonUtf8, pos, cur);
                }
                pos += width;
                cur = p.slide(cur, 1);
            }
        }
    }
}

/// `Space.chomp`.
pub(super) fn chomp<X>(p: &mut P<'_>, to_error: impl FnOnce(Space, Cursor) -> X) -> Result<(), X> {
    let (status, pos, cur) = eat_spaces(p, p.pos, p.cur);
    match status {
        None => {
            p.ok_at(pos, cur);
            Ok(())
        }
        Some(space) => p.cerr(to_error(space, cur)),
    }
}

/// `chompAndCheckIndent`.
pub(super) fn chomp_and_check_indent<X>(
    p: &mut P<'_>,
    to_space_error: impl FnOnce(Space, Cursor) -> X,
    to_indent_error: impl FnOnce(Cursor) -> X,
) -> Result<(), X> {
    let (status, pos, cur) = eat_spaces(p, p.pos, p.cur);
    match status {
        None if p.is_indented(cur) => {
            p.ok_at(pos, cur);
            Ok(())
        }
        None => {
            let x = to_indent_error(p.cur);
            p.cerr(x)
        }
        Some(space) => p.cerr(to_space_error(space, cur)),
    }
}

/// `checkIndent`: whether the spaces just chomped end indented, or
/// `to_error` where they began.
pub(super) fn check_indent<X>(
    p: &P<'_>,
    end: Cursor,
    to_error: impl FnOnce(Cursor) -> X,
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
    to_error: impl FnOnce(u32, Cursor) -> X,
) -> Result<(), X> {
    if p.cur.col == p.indent {
        Ok(())
    } else {
        Err(to_error(p.indent, p.cur))
    }
}

/// `checkFreshLine`.
pub(super) fn check_fresh_line<X>(p: &P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<(), X> {
    if p.cur.col == p.first() {
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

/// `docComment`.
pub(super) fn doc_comment<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Cursor) -> X,
    to_space_error: impl FnOnce(Space, Cursor) -> X,
) -> Result<(), X> {
    if !(p.peek(0) == Some(b'{') && p.peek(1) == Some(b'-') && p.peek(2) == Some(b'|')) {
        return Err(to_expectation(p.cur));
    }
    let (status, pos, cur) = eat_multi_comment(p, p.pos + 3, p.slide(p.cur, 3));
    match status {
        Multi::Good => {
            p.ok_at(pos, cur);
            Ok(())
        }
        Multi::Tab => p.cerr(to_space_error(Space::HasTab, cur)),
        Multi::NonUtf8 => p.cerr(to_space_error(Space::HasNonUtf8, cur)),
        Multi::Endless => {
            let x = to_space_error(Space::EndlessMultiComment, p.cur);
            p.cerr(x)
        }
    }
}

/// Runs `parser` with room on the stack, as GHC's growing stack gives
/// upstream's.
pub(super) fn deep<'s, T>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> T) -> T {
    stacker::maybe_grow(
        crate::parser::RED_ZONE,
        crate::parser::STACK_SEGMENT,
        || parser(p),
    )
}
