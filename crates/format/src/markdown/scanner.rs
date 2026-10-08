//! Cheapskate's parser state and the combinators its parsers share
//! (`Cheapskate/ParserCombinators.hs` and `Cheapskate/Util.hs`).
//!
//! Upstream's `<|>` backtracks fully, so a parser here is a function of a
//! `&mut Scanner` returning `Option`, and an alternative is tried on a
//! copy: [`Scanner::attempt`] puts the state back when it fails.

/// What is left of the subject, the column at which it starts, and the
/// character consumed last.
#[derive(Clone, Copy)]
pub(super) struct Scanner<'a> {
    rest: &'a str,
    /// Counted in characters from 1, as upstream's `Position`.
    pub(super) column: i64,
    last: Option<char>,
}

impl<'a> Scanner<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Scanner {
            rest: text,
            column: 1,
            last: None,
        }
    }

    /// What is left, unconsumed.
    pub(super) fn rest(&self) -> &'a str {
        self.rest
    }

    pub(super) fn at_end(&self) -> bool {
        self.rest.is_empty()
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    /// The character consumed last; upstream's `peekLastChar`.
    pub(super) fn last(&self) -> Option<char> {
        self.last
    }

    /// Consumes the first `len` bytes.
    fn advance(&mut self, len: usize) -> &'a str {
        let (taken, rest) = self.rest.split_at(len);
        for c in taken.chars() {
            self.column = if c == '\n' { 1 } else { self.column + 1 };
            self.last = Some(c);
        }
        self.rest = rest;
        taken
    }

    /// `p`, or nothing consumed when it fails.
    pub(super) fn attempt<T>(&mut self, p: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        let saved = *self;
        let result = p(self);
        if result.is_none() {
            *self = saved;
        }
        result
    }

    /// Whether `p` would succeed here, consuming nothing; upstream's
    /// `lookAhead` and, negated, `notFollowedBy`.
    pub(super) fn sees<T>(&self, p: impl FnOnce(&mut Self) -> Option<T>) -> bool {
        let mut copy = *self;
        p(&mut copy).is_some()
    }

    /// `p` as many times as it succeeds; upstream's `many`.
    pub(super) fn many<T>(&mut self, mut p: impl FnMut(&mut Self) -> Option<T>) -> Vec<T> {
        let mut items = Vec::new();
        while let Some(item) = self.attempt(&mut p) {
            items.push(item);
        }
        items
    }

    /// `p` until `end` succeeds, which is tried first; upstream's
    /// `manyTill`.
    pub(super) fn many_till<T, E>(
        &mut self,
        mut p: impl FnMut(&mut Self) -> Option<T>,
        mut end: impl FnMut(&mut Self) -> Option<E>,
    ) -> Option<Vec<T>> {
        let mut items = Vec::new();
        loop {
            if self.attempt(&mut end).is_some() {
                return Some(items);
            }
            items.push(p(self)?);
        }
    }

    pub(super) fn satisfy(&mut self, f: impl FnOnce(char) -> bool) -> Option<char> {
        let c = self.peek().filter(|&c| f(c))?;
        self.advance(c.len_utf8());
        Some(c)
    }

    pub(super) fn char(&mut self, c: char) -> Option<()> {
        self.satisfy(|x| x == c).map(drop)
    }

    pub(super) fn any_char(&mut self) -> Option<char> {
        self.satisfy(|_| true)
    }

    pub(super) fn string(&mut self, s: &str) -> Option<()> {
        self.rest.starts_with(s).then(|| {
            self.advance(s.len());
        })
    }

    pub(super) fn take_while(&mut self, mut f: impl FnMut(char) -> bool) -> &'a str {
        let len = self.rest.find(|c| !f(c)).unwrap_or(self.rest.len());
        self.advance(len)
    }

    pub(super) fn take_while1(&mut self, f: impl FnMut(char) -> bool) -> Option<&'a str> {
        Some(self.take_while(f)).filter(|t| !t.is_empty())
    }

    /// Everything left; upstream's `takeText`.
    pub(super) fn take_rest(&mut self) -> &'a str {
        self.advance(self.rest.len())
    }

    /// Up to `count` characters meeting `f`; upstream's `upToCountChars`.
    pub(super) fn up_to(&mut self, count: i64, f: impl Fn(char) -> bool) -> &'a str {
        let mut n = 0;
        let len = self
            .rest
            .char_indices()
            .find(|&(_, c)| {
                let stop = n >= count || !f(c);
                n += 1;
                stop
            })
            .map_or(self.rest.len(), |(i, _)| i);
        self.advance(len)
    }

    pub(super) fn end(&self) -> Option<()> {
        self.at_end().then_some(())
    }

    /// Spaces; upstream's `scanSpaces`.
    pub(super) fn spaces(&mut self) {
        self.take_while(|c| c == ' ');
    }

    /// Up to three spaces; upstream's `scanNonindentSpace`.
    pub(super) fn nonindent_space(&mut self) {
        self.up_to(3, |c| c == ' ');
    }

    /// Spaces to the end; upstream's `scanBlankline`.
    pub(super) fn blank_line(&mut self) -> Option<()> {
        self.spaces();
        self.end()
    }

    /// Spaces, and a newline and spaces if one follows; upstream's
    /// `scanSpnl`.
    pub(super) fn spnl(&mut self) {
        self.spaces();
        if self.char('\n').is_some() {
            self.spaces();
        }
    }

    /// Exactly the spaces up to `column`, if it is ahead; upstream's
    /// `scanSpacesToColumn`.
    pub(super) fn spaces_to_column(&mut self, column: i64) -> Option<()> {
        for _ in 0..column - self.column {
            self.char(' ')?;
        }
        Some(())
    }
}

/// Cheapskate's `isWhitespace`.
pub(super) fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Cheapskate's `isEscapable`: ASCII symbols and punctuation.
pub(super) fn is_escapable(c: char) -> bool {
    c.is_ascii_punctuation()
}
