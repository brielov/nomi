//! A port of the parser in upstream's `Json/Decode.hs`. A value is parsed
//! as `oneOf Start [string, object, array, int, true, false, null]`; the
//! alternatives all fail without consuming input unless their first byte
//! matches, so dispatching on that byte reproduces the backtracking, and
//! any failure after it is final.

use nomi_base::{Compat, ghc};

use super::cursor::{Cursor, Position, Region};

/// A parsed value and the region upstream records for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value<'a> {
    pub region: Region,
    pub kind: Kind<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind<'a> {
    Array(Vec<Value<'a>>),
    /// Fields in source order, duplicates kept.
    Object(Vec<(Snippet<'a>, Value<'a>)>),
    String(Snippet<'a>),
    /// Wrapped to 64 bits, as Haskell's `Int` does.
    Int(i64),
    True,
    False,
    Null,
}

/// Dropped with a loop, not recursion, so that nesting as deep as the
/// parser takes doesn't overflow the stack on the way out.
impl Drop for Value<'_> {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_children(&mut self.kind, &mut pending);
        while let Some(mut value) = pending.pop() {
            take_children(&mut value.kind, &mut pending);
        }
    }
}

fn take_children<'a>(kind: &mut Kind<'a>, pending: &mut Vec<Value<'a>>) {
    match kind {
        Kind::Array(entries) => pending.append(entries),
        Kind::Object(fields) => pending.extend(fields.drain(..).map(|(_, v)| v)),
        _ => {}
    }
}

/// The bytes between a string's quotes, escapes left as written, and the
/// position of the first of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snippet<'a> {
    pub bytes: &'a [u8],
    pub start: Cursor,
}

impl Snippet<'_> {
    /// `snippetToRegion`: the bytes' region, one column per byte.
    pub fn region(&self) -> Region {
        Region {
            start: self.start.position(),
            end: self.start.slide(self.bytes.len()).position(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub at: Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// No value where one should start.
    Start,
    /// No key, or no `}`, after `{`.
    ObjectField,
    ObjectColon,
    ObjectEnd,
    ArrayEnd,
    String(StringProblem),
    NoLeadingZeros,
    NoFloats,
    /// Something after the value.
    BadEnd,
    /// 0.19.1 dies with `Need UTF-8 encoded input. Ran into unrecognized
    /// bits.` on a string byte that can't start a UTF-8 sequence.
    Crash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringProblem {
    BadStringEnd,
    BadStringControlChar,
    BadStringEscapeChar,
    BadStringEscapeHex,
    BadStringNotUtf8,
}

/// Parses a whole file as `compat`'s elm does.
pub fn parse(src: &[u8], compat: Compat) -> Result<Value<'_>, ParseError> {
    let mut p = Parser {
        src,
        pos: 0,
        cur: Cursor::start(compat),
        compat,
    };
    p.spaces();
    let value = p.value()?;
    p.spaces();
    if p.pos == src.len() {
        Ok(value)
    } else {
        Err(p.error(ParseErrorKind::BadEnd))
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    cur: Cursor,
    compat: Compat,
}

type Parsed<T> = Result<T, ParseError>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        self.error_at(self.cur, kind)
    }

    fn error_at(&self, cur: Cursor, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            at: cur.position(),
        }
    }

    fn advance(&mut self, bytes: usize) {
        self.pos += bytes;
        self.cur = self.cur.slide(bytes);
    }

    /// `word1`: one expected byte, or `kind` here.
    fn byte(&mut self, byte: u8, kind: ParseErrorKind) -> Parsed<()> {
        if self.peek() == Some(byte) {
            self.advance(1);
            Ok(())
        } else {
            Err(self.error(kind))
        }
    }

    fn spaces(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' => self.cur = self.cur.slide(1),
                b'\n' => self.cur = self.cur.newline(),
                b'\r' => {}
                _ => return,
            }
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Parsed<Value<'a>> {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            let start = self.cur;
            let kind = match self.peek() {
                Some(b'"') => Kind::String(self.string(ParseErrorKind::Start)?),
                Some(b'{') => self.object()?,
                Some(b'[') => self.array()?,
                Some(b'0'..=b'9') => self.int()?,
                _ => {
                    if self.keyword(b"true") {
                        Kind::True
                    } else if self.keyword(b"false") {
                        Kind::False
                    } else if self.keyword(b"null") {
                        Kind::Null
                    } else {
                        return Err(self.error_at(start, ParseErrorKind::Start));
                    }
                }
            };
            Ok(Value {
                region: Region {
                    start: start.position(),
                    end: self.cur.position(),
                },
                kind,
            })
        })
    }

    fn object(&mut self) -> Parsed<Kind<'a>> {
        self.byte(b'{', ParseErrorKind::Start)?;
        self.spaces();
        let mut fields = Vec::new();
        match self.peek() {
            Some(b'"') => fields.push(self.field()?),
            Some(b'}') => {
                self.advance(1);
                return Ok(Kind::Object(fields));
            }
            _ => return Err(self.error(ParseErrorKind::ObjectField)),
        }
        loop {
            self.spaces();
            match self.peek() {
                Some(b',') => {
                    self.advance(1);
                    self.spaces();
                    fields.push(self.field()?);
                }
                Some(b'}') => {
                    self.advance(1);
                    return Ok(Kind::Object(fields));
                }
                _ => return Err(self.error(ParseErrorKind::ObjectEnd)),
            }
        }
    }

    fn field(&mut self) -> Parsed<(Snippet<'a>, Value<'a>)> {
        let key = self.string(ParseErrorKind::ObjectField)?;
        self.spaces();
        self.byte(b':', ParseErrorKind::ObjectColon)?;
        self.spaces();
        Ok((key, self.value()?))
    }

    fn array(&mut self) -> Parsed<Kind<'a>> {
        self.byte(b'[', ParseErrorKind::Start)?;
        self.spaces();
        if self.peek() == Some(b']') {
            self.advance(1);
            return Ok(Kind::Array(Vec::new()));
        }
        let mut entries = vec![self.value()?];
        loop {
            self.spaces();
            match self.peek() {
                Some(b',') => {
                    self.advance(1);
                    self.spaces();
                    entries.push(self.value()?);
                }
                Some(b']') => {
                    self.advance(1);
                    return Ok(Kind::Array(entries));
                }
                _ => return Err(self.error(ParseErrorKind::ArrayEnd)),
            }
        }
    }

    /// `pString`: `start` if no string starts here, else the string or
    /// the problem in it, where the problem is.
    fn string(&mut self, start: ParseErrorKind) -> Parsed<Snippet<'a>> {
        if self.peek() != Some(b'"') {
            return Err(self.error(start));
        }
        let first = self.pos + 1;
        let first_cur = self.cur.slide(1);
        let mut pos = first;
        let mut cur = first_cur;
        let problem = |cur: Cursor, problem| ParseError {
            kind: ParseErrorKind::String(problem),
            at: cur.position(),
        };
        loop {
            let Some(&byte) = self.src.get(pos) else {
                return Err(problem(cur, StringProblem::BadStringEnd));
            };
            match byte {
                b'"' => {
                    let snippet = Snippet {
                        bytes: &self.src[first..pos],
                        start: first_cur,
                    };
                    self.pos = pos + 1;
                    self.cur = cur.slide(1);
                    return Ok(snippet);
                }
                b'\n' => return Err(problem(cur, StringProblem::BadStringEnd)),
                b'\\' => match self.src.get(pos + 1) {
                    None => {
                        let at = match self.compat {
                            Compat::V0_19_1 => cur.down(),
                            Compat::V0_19_2 | Compat::V0_19_3 => cur.slide(1),
                        };
                        return Err(problem(at, StringProblem::BadStringEnd));
                    }
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                        pos += 2;
                        cur = cur.slide(2);
                    }
                    Some(b'u') => {
                        let hex = self.src.get(pos + 2..pos + 6);
                        if !hex.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) {
                            return Err(problem(cur, StringProblem::BadStringEscapeHex));
                        }
                        pos += 6;
                        cur = cur.slide(6);
                    }
                    Some(_) => return Err(problem(cur, StringProblem::BadStringEscapeChar)),
                },
                0..0x20 => return Err(problem(cur, StringProblem::BadStringControlChar)),
                _ => {
                    let width = match self.compat {
                        Compat::V0_19_1 => match char_width(byte) {
                            Some(width) => width,
                            None => return Err(self.error_at(cur, ParseErrorKind::Crash)),
                        },
                        Compat::V0_19_2 | Compat::V0_19_3 => match utf8_width(&self.src[pos..]) {
                            Some(width) => width,
                            None => return Err(problem(cur, StringProblem::BadStringNotUtf8)),
                        },
                    };
                    // 0.19.1 trusts the lead byte and may step past the end,
                    // which then ends the string.
                    pos += width;
                    cur = cur.slide(1);
                }
            }
        }
    }

    /// `pInt`, called on a digit.
    fn int(&mut self) -> Parsed<Kind<'a>> {
        let digits = self.src[self.pos..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if self.src[self.pos] == b'0' {
            return match self.src.get(self.pos + 1) {
                Some(b'0'..=b'9') => {
                    Err(self.error_at(self.cur.slide(1), ParseErrorKind::NoLeadingZeros))
                }
                Some(b'.') => Err(self.error_at(self.cur.slide(1), ParseErrorKind::NoFloats)),
                _ => {
                    self.advance(1);
                    Ok(Kind::Int(0))
                }
            };
        }
        let n = self.src[self.pos..self.pos + digits]
            .iter()
            .fold(0i64, |n, d| {
                n.wrapping_mul(10).wrapping_add(i64::from(d - b'0'))
            });
        if let Some(b'.' | b'e' | b'E') = self.src.get(self.pos + digits) {
            return Err(self.error_at(self.cur.slide(digits), ParseErrorKind::NoFloats));
        }
        self.advance(digits);
        Ok(Kind::Int(n))
    }

    /// The keyword `word`, not followed by what could continue a name.
    fn keyword(&mut self, word: &[u8]) -> bool {
        let rest = &self.src[self.pos..];
        if !rest.starts_with(word) || continues_name(&rest[word.len()..], self.compat) {
            return false;
        }
        self.advance(word.len());
        true
    }
}

/// 0.19.1's `getCharWidth`: the width a lead byte announces, or `None`
/// where elm crashes.
fn char_width(lead: u8) -> Option<usize> {
    match lead {
        0..0x80 => Some(1),
        0x80..0xC0 | 0xF8.. => None,
        0xC0..0xE0 => Some(2),
        0xE0..0xF0 => Some(3),
        0xF0..0xF8 => Some(4),
    }
}

/// `skipUtf8`: the width of the well-formed UTF-8 character `bytes` start
/// with, if they start with one. The width comes from the lead byte alone,
/// so a stray continuation byte reads as a bad two-byte sequence.
fn utf8_width(bytes: &[u8]) -> Option<usize> {
    let width = match bytes.first()? {
        0 => return None,
        0x01..0x80 => 1,
        0x80..0xE0 => 2,
        0xE0..0xF0 => 3,
        0xF0.. => 4,
    };
    let char = bytes.get(..width)?;
    std::str::from_utf8(char).ok().map(|_| width)
}

/// Whether `rest` starts with a character that would continue a name:
/// `[A-Za-z0-9_]`, or a letter outside ASCII.
fn continues_name(rest: &[u8], compat: Compat) -> bool {
    let Some(&lead) = rest.first() else {
        return false;
    };
    if lead.is_ascii_alphanumeric() || lead == b'_' {
        return true;
    }
    let width = match lead {
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        0xF0..0xF8 => 4,
        _ => return false,
    };
    let c = match compat {
        Compat::V0_19_1 => unchecked_char(rest, width),
        Compat::V0_19_2 | Compat::V0_19_3 => rest
            .get(..width)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|s| s.chars().next()),
    };
    c.is_some_and(ghc::is_alpha)
}

/// 0.19.1's `chr2`, `chr3` and `chr4`, which assemble a character from a
/// lead byte and its continuation bytes without checking them. Upstream
/// reads past the end of the input for a truncated sequence; here those
/// bytes are zero. A result that is no character is no letter either.
fn unchecked_char(bytes: &[u8], width: usize) -> Option<char> {
    let byte = |i: usize| i64::from(bytes.get(i).copied().unwrap_or(0));
    let tail = |i: usize, shift: u32| (byte(i) - 0x80) << shift;
    let code = match width {
        2 => ((byte(0) - 0xC0) << 6) + tail(1, 0),
        3 => ((byte(0) - 0xE0) << 12) + tail(1, 6) + tail(2, 0),
        _ => ((byte(0) - 0xF0) << 18) + tail(1, 12) + tail(2, 6) + tail(3, 0),
    };
    u32::try_from(code).ok().and_then(char::from_u32)
}
