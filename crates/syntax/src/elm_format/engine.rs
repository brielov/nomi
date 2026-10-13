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

/// A row and column, as upstream's `Word16`s, which wrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pos {
    pub(super) row: u16,
    pub(super) col: u16,
}

impl Pos {
    /// `n` columns right.
    pub(super) fn right(self, n: u16) -> Pos {
        Pos {
            row: self.row,
            col: self.col.wrapping_add(n),
        }
    }

    /// The start of the next row.
    pub(super) fn next_row(self) -> Pos {
        Pos {
            row: self.row.wrapping_add(1),
            col: 1,
        }
    }
}

/// Why a parser failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fail {
    /// A parse error here.
    At(Pos),
    /// A Haskell `error`, which nothing catches.
    Crash(&'static str),
}

pub(super) type R<T> = Result<T, Fail>;

/// An alternative for [`P::alts`] and [`P::choice`], or a parser the
/// grammar passes around.
pub(super) type Alt<'a, 's, T> = &'a mut dyn FnMut(&mut P<'s>) -> R<T>;

/// `State`, with the newline stack elm-format adds, and whether the
/// parser running has consumed.
pub(super) struct P<'s> {
    src: &'s str,
    pub(super) pos: usize,
    pub(super) at: Pos,
    indent: u16,
    newline: Vec<bool>,
    consumed: bool,
}

/// What [`P::restore`] puts back. Of the newline stack, only its height
/// and top: inside an alternative every push is popped before the
/// alternative ends, and only the top is ever set, so what lies below
/// the height an alternative started at is never touched.
#[derive(Clone, Copy)]
struct Saved {
    pos: usize,
    at: Pos,
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
            at: Pos { row: 1, col: 1 },
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

    fn save(&self) -> Saved {
        Saved {
            pos: self.pos,
            at: self.at,
            indent: self.indent,
            height: self.newline.len(),
            top: self.newline.last().copied().unwrap_or(false),
            consumed: self.consumed,
        }
    }

    fn restore(&mut self, saved: Saved) {
        debug_assert!(self.newline.len() >= saved.height);
        self.pos = saved.pos;
        self.at = saved.at;
        self.indent = saved.indent;
        self.newline.truncate(saved.height);
        if let Some(top) = self.newline.last_mut() {
            *top = saved.top;
        }
        self.consumed = saved.consumed;
    }

    /// A failure here: `parserFail`, and `guard False`.
    pub(super) fn fail<T>(&self) -> R<T> {
        Err(Fail::At(self.at))
    }

    /// A consumed success, `bytes` on, at `to`.
    pub(super) fn advance(&mut self, bytes: usize, to: Pos) {
        self.pos += bytes;
        self.at = to;
        self.consumed = true;
    }

    /// A consumed failure at `at`.
    pub(super) fn cerr<T>(&mut self, at: Pos) -> R<T> {
        self.consumed = true;
        Err(Fail::At(at))
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
        let to = match c {
            '\n' => self.at.next_row(),
            '\t' => self
                .at
                .right(8u16.wrapping_sub(self.at.col.wrapping_sub(1) % 8)),
            _ => self.at.right(1),
        };
        self.advance(c.len_utf8(), to);
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

    // COMBINATORS

    /// One alternative, from `saved` with a clear consumed flag: what it
    /// gave if it succeeded or failed having consumed (`Ok`), or its
    /// failure, with the state put back, if it failed without (`Err`).
    fn try_alt<T>(
        &mut self,
        saved: Saved,
        alt: impl FnOnce(&mut P<'s>) -> R<T>,
    ) -> Result<R<T>, Fail> {
        self.consumed = false;
        match alt(self) {
            Ok(value) => {
                self.consumed |= saved.consumed;
                Ok(Ok(value))
            }
            Err(e @ Fail::At(..)) if !self.consumed => {
                self.restore(saved);
                Err(e)
            }
            Err(e) => Ok(Err(e)),
        }
    }

    /// `<|>`: `a`, or `b` from where `a` started if `a` failed without
    /// consuming. Unlike Parsec's, the adapter keeps no error from an
    /// empty success, and when both fail it reports where `b` did.
    pub(super) fn alt<T>(
        &mut self,
        a: impl FnOnce(&mut P<'s>) -> R<T>,
        b: impl FnOnce(&mut P<'s>) -> R<T>,
    ) -> R<T> {
        let saved = self.save();
        match self.try_alt(saved, a) {
            Ok(result) => result,
            Err(_) => self.try_alt(saved, b).unwrap_or_else(Err),
        }
    }

    /// `a <|> b <|> …`, which nests to the right: the first alternative
    /// to succeed or to fail having consumed, or where the last failed.
    pub(super) fn alts<T>(&mut self, alts: &mut [Alt<'_, 's, T>]) -> R<T> {
        let saved = self.save();
        let mut last = Fail::At(self.at);
        for alt in alts {
            match self.try_alt(saved, alt) {
                Ok(result) => return result,
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// `choice`: [`Self::alt`] folded over `alts` and ending in
    /// `parserFail`, so when every alternative fails without consuming it
    /// fails where it started.
    pub(super) fn choice<T>(&mut self, alts: &mut [Alt<'_, 's, T>]) -> R<T> {
        let saved = self.save();
        for alt in alts {
            if let Ok(result) = self.try_alt(saved, alt) {
                return result;
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
        let (at, consumed) = (self.at, self.consumed);
        self.restore(saved);
        match result {
            Ok(_) if consumed => Err(Fail::At(at)),
            Err(e @ Fail::Crash(_)) => Err(e),
            Ok(_) | Err(Fail::At(..)) => Ok(()),
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
        let first = self.try_alt(saved, |p| {
            p.attempt(|p| p.look_ahead(check))?;
            parser(p)
        });
        match first {
            Ok(result) => result,
            Err(_) => self
                .try_alt(saved, |p| p.attempt(parser))
                .unwrap_or_else(Err),
        }
    }

    // INDENTATION

    /// `withPos`: `parser` with the indent at the current column.
    pub(super) fn with_pos<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        let old = self.indent;
        self.indent = self.at.col;
        let value = parser(self)?;
        self.indent = old;
        Ok(value)
    }

    /// `checkIndent`: here is the indent's column.
    pub(super) fn check_indent(&self) -> R<()> {
        if self.indent == self.at.col {
            Ok(())
        } else {
            self.fail()
        }
    }

    /// `indented`: here is right of the indent.
    pub(super) fn indented(&self) -> R<()> {
        if self.at.col <= self.indent {
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

    /// `trackNewline`: `parser`, with a newline context of its own.
    pub(super) fn track_newline<T>(&mut self, parser: impl FnOnce(&mut P<'s>) -> R<T>) -> R<T> {
        Ok(self.track_newline_seen(parser)?.0)
    }

    /// [`P::track_newline`], and whether a newline `parser` parsed was
    /// noted, which counts the newlines of the `trackNewline`s inside it.
    pub(super) fn track_newline_seen<T>(
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
