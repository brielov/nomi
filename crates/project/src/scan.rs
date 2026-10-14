//! What upstream's `customString` parsers share: a position in a string's
//! bytes and the cursor there, and `fromSnippet`'s rule that a parse must
//! take every byte. Failures are the cursor where upstream reports them.

use nomi_base::{Compat, ghc, utf8};

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

/// The width of the character `rest` starts with if it is good: ASCII by
/// `ascii`, anything else by `unicode`; zero if it isn't. This is
/// `AST/Prim/Variable.hs`'s `chomp` for 0.19.2 and later, which decodes
/// strict UTF-8 within the input, and 0.19.1's `get*WidthHelp`, which
/// assembles the character from its lead byte's width unchecked.
pub(crate) fn chomp(
    rest: &[u8],
    ascii: impl Fn(u8) -> bool,
    unicode: impl Fn(char) -> bool,
    compat: Compat,
) -> usize {
    let Some(&lead) = rest.first() else {
        return 0;
    };
    if ascii(lead) {
        return 1;
    }
    let width = match lead {
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        0xF0..0xF8 => 4,
        _ => return 0,
    };
    let c = match compat {
        Compat::V0_19_1 => unchecked_char(rest, width),
        Compat::V0_19_2 | Compat::V0_19_3 => rest
            .get(..width)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|s| s.chars().next()),
    };
    if c.is_some_and(unicode) { width } else { 0 }
}

/// `chompInner`: a character that continues a name, `[A-Za-z0-9_]` or a
/// letter outside ASCII.
pub(crate) fn inner(rest: &[u8], compat: Compat) -> usize {
    chomp(
        rest,
        |b| b.is_ascii_alphanumeric() || b == b'_',
        ghc::is_alpha,
        compat,
    )
}

/// `chompUpper`: a character that starts a capitalized name.
pub(crate) fn upper(rest: &[u8], compat: Compat) -> usize {
    chomp(rest, |b| b.is_ascii_uppercase(), ghc::is_upper, compat)
}

/// 0.19.1's `chr2`, `chr3` and `chr4` ([`utf8::unchecked_code`]). A
/// result that is no character is no letter either.
fn unchecked_char(bytes: &[u8], width: usize) -> Option<char> {
    u32::try_from(utf8::unchecked_code(bytes, width))
        .ok()
        .and_then(char::from_u32)
}
