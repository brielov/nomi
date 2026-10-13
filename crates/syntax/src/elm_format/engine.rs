//! `Parse/ParsecAdapter.hs`, `Parse/Primitives.hs` and `Parse/State.hs`:
//! Parsec's combinators over elm's continuation-passing parser.
//!
//! As in the port of elm's parser, a parser is a function of the state
//! returning a `Result`, and whether it consumed is a flag on the state
//! that every primitive which advances sets and nothing clears but the
//! combinators that may try something else. A failure carries only its
//! position: the one upstream's error ends up reporting is the row and
//! column handed to the last error continuation, which is what each
//! combinator here passes on.

use crate::parser::{RED_ZONE, STACK_SEGMENT};

/// Why a parser failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fail {
    /// A parse error at a row and column.
    At(u16, u16),
    /// A Haskell `error`, which nothing catches.
    Crash(&'static str),
}

pub(super) type R<T> = Result<T, Fail>;

/// A parser the grammar passes around.
pub(super) type Dyn<'a, 's, T> = &'a mut dyn FnMut(&mut P<'s>) -> R<T>;

/// `State`, with the newline stack elm-format adds, and whether the
/// parser running has consumed.
pub(super) struct P<'s> {
    src: &'s str,
    pub(super) pos: usize,
    pub(super) row: u16,
    pub(super) col: u16,
    indent: u16,
    newline: Vec<bool>,
    pub(super) consumed: bool,
}

/// What [`P::restore`] puts back. Of the newline stack, only its height
/// and top: inside an alternative every push is popped before the
/// alternative ends, and only the top is ever set, so what lies below
/// the height an alternative started at is never touched.
#[derive(Clone, Copy)]
pub(super) struct Saved {
    pos: usize,
    row: u16,
    col: u16,
    indent: u16,
    height: usize,
    top: bool,
    consumed: bool,
}

impl<'s> P<'s> {
    /// `runParserT` from `State.init`: the newline stack holds one `False`.
    pub(super) fn new(src: &'s str) -> P<'s> {
        P {
            src,
            pos: 0,
            row: 1,
            col: 1,
            indent: 1,
            newline: vec![false],
            consumed: false,
        }
    }

    pub(super) fn src(&self) -> &'s str {
        self.src
    }

    /// The byte `off` past the position, if there is one.
    pub(super) fn peek(&self, off: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + off).copied()
    }

    pub(super) fn save(&self) -> Saved {
        Saved {
            pos: self.pos,
            row: self.row,
            col: self.col,
            indent: self.indent,
            height: self.newline.len(),
            top: self.newline.last().copied().unwrap_or(false),
            consumed: self.consumed,
        }
    }

    pub(super) fn restore(&mut self, saved: Saved) {
        debug_assert!(self.newline.len() >= saved.height);
        self.pos = saved.pos;
        self.row = saved.row;
        self.col = saved.col;
        self.indent = saved.indent;
        self.newline.truncate(saved.height);
        if let Some(top) = self.newline.last_mut() {
            *top = saved.top;
        }
        self.consumed = saved.consumed;
    }

    /// A failure here.
    pub(super) fn fail<T>(&self) -> R<T> {
        Err(Fail::At(self.row, self.col))
    }

    /// A consumed success, `bytes` on at `col`, in the same row.
    pub(super) fn advance(&mut self, bytes: usize, col: u16) {
        self.pos += bytes;
        self.col = col;
        self.consumed = true;
    }

    /// A consumed success, `bytes` on at `row` and `col`.
    pub(super) fn advance_to(&mut self, bytes: usize, row: u16, col: u16) {
        self.pos += bytes;
        self.row = row;
        self.col = col;
        self.consumed = true;
    }

    /// A consumed failure at `row` and `col`.
    pub(super) fn cerr<T>(&mut self, row: u16, col: u16) -> R<T> {
        self.consumed = true;
        Err(Fail::At(row, col))
    }

    // PRIMITIVES

    /// `satisfy`: the character here, if `f` holds of it. Columns count
    /// as `updatePos` counts them, a tab to the next multiple of eight.
    pub(super) fn satisfy(&mut self, f: impl FnOnce(char) -> bool) -> R<char> {
        let Some(c) = self.src[self.pos..].chars().next() else {
            return self.fail();
        };
        // `extractChar` refuses the two noncharacters at the end of the
        // Basic Multilingual Plane, by `error`.
        if matches!(c, '\u{FFFE}' | '\u{FFFF}') {
            return Err(Fail::Crash("invalid utf-8"));
        }
        if !f(c) {
            return self.fail();
        }
        let (row, col) = match c {
            '\n' => (self.row.wrapping_add(1), 1),
            '\t' => {
                let col = self.col;
                (
                    self.row,
                    col.wrapping_add(8).wrapping_sub(col.wrapping_sub(1) % 8),
                )
            }
            _ => (self.row, self.col.wrapping_add(1)),
        };
        self.advance_to(c.len_utf8(), row, col);
        Ok(c)
    }

    pub(super) fn char(&mut self, c: char) -> R<()> {
        self.satisfy(|d| d == c).map(drop)
    }

    pub(super) fn any_char(&mut self) -> R<char> {
        self.satisfy(|_| true)
    }

    /// `string`: each character in turn, so a partial match fails having
    /// consumed, where it stopped matching.
    pub(super) fn string(&mut self, s: &str) -> R<()> {
        for c in s.chars() {
            self.char(c)?;
        }
        Ok(())
    }

    /// `digit`: GHC's `isDigit`, which is ASCII's.
    pub(super) fn digit(&mut self) -> R<char> {
        self.satisfy(|c| c.is_ascii_digit())
    }

    /// `parserFail`, and `guard False`.
    pub(super) fn parser_fail<T>(&self) -> R<T> {
        self.fail()
    }

    // COMBINATORS

    /// `<|>`: `a`, or `b` from where `a` started if `a` failed without
    /// consuming. Unlike Parsec's, the adapter keeps no error from an
    /// empty success, and when both fail it reports where `b` did.
    pub(super) fn alt<T>(
        &mut self,
        a: impl FnOnce(&mut P<'s>) -> R<T>,
        b: impl FnOnce(&mut P<'s>) -> R<T>,
    ) -> R<T> {
        let saved = self.save();
        self.consumed = false;
        match a(self) {
            Ok(value) => {
                self.consumed |= saved.consumed;
                Ok(value)
            }
            Err(Fail::At(..)) if !self.consumed => {
                self.restore(saved);
                self.consumed = false;
                self.last(saved, b)
            }
            Err(e) => Err(e),
        }
    }

    /// The last alternative, from `saved`.
    fn last<T>(&mut self, saved: Saved, b: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        match b(self) {
            Ok(value) => {
                self.consumed |= saved.consumed;
                Ok(value)
            }
            Err(e @ Fail::At(..)) if !self.consumed => {
                self.restore(saved);
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// `a <|> b <|> …`, which nests to the right: the first alternative
    /// to succeed or to fail having consumed, or where the last failed.
    pub(super) fn alts<T>(&mut self, alts: &mut [Dyn<'_, 's, T>]) -> R<T> {
        let saved = self.save();
        let mut last = Fail::At(self.row, self.col);
        for alt in alts {
            self.consumed = false;
            match alt(self) {
                Ok(value) => {
                    self.consumed |= saved.consumed;
                    return Ok(value);
                }
                Err(e @ Fail::At(..)) if !self.consumed => {
                    self.restore(saved);
                    last = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// `choice`: [`Self::alt`] folded over `alts` and ending in
    /// `parserFail`, so when every alternative fails without consuming it
    /// fails where it started.
    pub(super) fn choice<T>(&mut self, alts: &mut [Dyn<'_, 's, T>]) -> R<T> {
        let saved = self.save();
        for alt in alts {
            self.consumed = false;
            match alt(self) {
                Ok(value) => {
                    self.consumed |= saved.consumed;
                    return Ok(value);
                }
                Err(Fail::At(..)) if !self.consumed => self.restore(saved),
                Err(e) => return Err(e),
            }
        }
        self.fail()
    }

    /// `option`: `parser`, or nothing where it started if it failed
    /// without consuming.
    pub(super) fn option<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<Option<T>> {
        self.alt(|p| parser(p).map(Some), |_| Ok(None))
    }

    /// `try`: `parser`, whose failures count as not consuming.
    pub(super) fn attempt<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        let outer = self.consumed;
        self.consumed = false;
        match parser(self) {
            Ok(value) => {
                self.consumed |= outer;
                Ok(value)
            }
            Err(e @ Fail::At(..)) => {
                self.consumed = outer;
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// `lookAhead`: `parser`, then back where it started if it succeeded.
    pub(super) fn look_ahead<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        let saved = self.save();
        let value = parser(self)?;
        self.restore(saved);
        Ok(value)
    }

    /// `notFollowedBy`: fails without consuming, after what `parser`
    /// matched, if it matched something; otherwise succeeds where it
    /// started.
    pub(super) fn not_followed_by<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<()> {
        let saved = self.save();
        self.consumed = false;
        let result = parser(self);
        let (row, col, consumed) = (self.row, self.col, self.consumed);
        self.restore(saved);
        match result {
            Ok(_) if consumed => Err(Fail::At(row, col)),
            Err(e @ Fail::Crash(_)) => Err(e),
            _ => Ok(()),
        }
    }

    /// `eof`: `notFollowedBy anyToken`, so it fails after the next
    /// character.
    pub(super) fn eof(&mut self) -> R<()> {
        self.not_followed_by(P::any_char)
    }

    /// `many`, counting. A parser that succeeds without consuming is
    /// Parsec's runtime error.
    pub(super) fn many<T>(&mut self, mut parser: impl FnMut(&mut P<'s>) -> R<T>) -> R<usize> {
        let mut n = 0;
        loop {
            let saved = self.save();
            self.consumed = false;
            match parser(self) {
                Ok(_) if !self.consumed => {
                    return Err(Fail::Crash(
                        "combinator 'many' is applied to a parser that accepts an empty string",
                    ));
                }
                Ok(_) => {
                    self.consumed = true;
                    n += 1;
                }
                Err(Fail::At(..)) if !self.consumed => {
                    self.restore(saved);
                    return Ok(n);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// `many1`, counting.
    pub(super) fn many1<T>(&mut self, mut parser: impl FnMut(&mut P<'s>) -> R<T>) -> R<usize> {
        parser(self)?;
        Ok(1 + self.many(parser)?)
    }

    /// `commitIf check p`: `(try (lookAhead check) >> p) <|> try p`.
    pub(super) fn commit_if<T>(
        &mut self,
        check: impl FnOnce(&mut P<'s>) -> R<()>,
        mut parser: impl FnMut(&mut P<'s>) -> R<T>,
    ) -> R<T> {
        let saved = self.save();
        self.consumed = false;
        let first = match self.attempt(|p| p.look_ahead(check)) {
            Ok(()) => parser(self),
            Err(e) => Err(e),
        };
        match first {
            Ok(value) => {
                self.consumed |= saved.consumed;
                Ok(value)
            }
            Err(Fail::At(..)) if !self.consumed => {
                self.restore(saved);
                self.consumed = false;
                self.last(saved, |p| p.attempt(parser))
            }
            Err(e) => Err(e),
        }
    }

    // INDENTATION

    /// `withPos`: `parser` with the indent at the current column.
    pub(super) fn with_pos<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        let old = self.indent;
        self.indent = self.col;
        let value = parser(self)?;
        self.indent = old;
        Ok(value)
    }

    /// `checkIndent`: here is the indent's column.
    pub(super) fn check_indent(&self) -> R<()> {
        if self.indent == self.col {
            Ok(())
        } else {
            self.fail()
        }
    }

    /// `indented`: here is right of the indent.
    pub(super) fn indented(&self) -> R<()> {
        if self.col <= self.indent {
            self.fail()
        } else {
            Ok(())
        }
    }

    // NEWLINES

    /// `updateState setNewline`.
    pub(super) fn set_newline(&mut self) {
        if let Some(top) = self.newline.last_mut() {
            *top = true;
        }
    }

    /// `trackNewline`: `parser`, and whether a newline it parsed was
    /// noted, which counts the newlines of `trackNewline`s inside it.
    pub(super) fn track_newline<T>(
        &mut self,
        parser: impl FnOnce(&mut P<'s>) -> R<T>,
    ) -> R<(T, bool)> {
        self.newline.push(false);
        let value = parser(self)?;
        let saw = self.newline.last().copied().unwrap_or(false);
        // `popNewlineContext`, which leaves the last context alone.
        if self.newline.len() > 1 {
            let last = self.newline.pop().unwrap_or(false);
            if let Some(next) = self.newline.last_mut() {
                *next |= last;
            }
        }
        Ok((value, saw))
    }
}

/// Runs `parser` with room on the stack, as GHC's growing stack gives
/// upstream's.
pub(super) fn deep<'s, T>(p: &mut P<'s>, parser: impl FnOnce(&mut P<'s>) -> T) -> T {
    stacker::maybe_grow(RED_ZONE, STACK_SEGMENT, || parser(p))
}
