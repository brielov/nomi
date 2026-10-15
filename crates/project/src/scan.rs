//! What upstream's `customString` parsers share: a position in a string's
//! bytes and the cursor there, and `fromSnippet`'s rule that a parse must
//! take every byte. Failures are the cursor where upstream reports them.

use nomi_base::Compat;

use crate::json::{Cursor, Position};

pub(crate) struct Scan<'a> {
    bytes: &'a [u8],
    pos: usize,
    cursor: Cursor,
    compat: Compat,
}

impl<'a> Scan<'a> {
    pub(crate) fn new(bytes: &'a [u8], start: Cursor, compat: Compat) -> Scan<'a> {
        Scan {
            bytes,
            pos: 0,
            cursor: start,
            compat,
        }
    }

    pub(crate) fn compat(&self) -> Compat {
        self.compat
    }

    pub(crate) fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Past `n` bytes that are `n` columns.
    pub(crate) fn advance(&mut self, n: usize) {
        self.skip(n, n);
    }

    /// Past `bytes` bytes that upstream counts as `columns` columns.
    pub(crate) fn skip(&mut self, bytes: usize, columns: usize) {
        self.pos += bytes;
        self.cursor = self.cursor.slide(columns);
    }

    /// `word1`: this byte, or a failure where it isn't.
    pub(crate) fn byte(&mut self, byte: u8) -> Result<(), Cursor> {
        if self.peek() == Some(byte) {
            self.advance(1);
            Ok(())
        } else {
            Err(self.cursor)
        }
    }

    /// `toOk`: the end of the string, or a failure where the parse stopped.
    pub(crate) fn finish(&self) -> Result<(), Cursor> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(self.cursor)
        }
    }
}

/// `fromSnippet`: `parse` from `start` over the whole of `bytes`, failing
/// where it stops or, if it stops short of the end, there.
pub(crate) fn whole<T>(
    bytes: &[u8],
    start: Cursor,
    compat: Compat,
    parse: impl FnOnce(&mut Scan<'_>) -> Result<T, Cursor>,
) -> Result<T, Position> {
    let mut scan = Scan::new(bytes, start, compat);
    let parsed = parse(&mut scan).and_then(|t| scan.finish().map(|()| t));
    parsed.map_err(Cursor::position)
}
