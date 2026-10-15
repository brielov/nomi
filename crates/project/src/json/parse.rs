//! A port of the parser in upstream's `Json/Decode.hs`. A value is parsed
//! as `oneOf Start [string, object, array, int, true, false, null]`; the
//! alternatives all fail without consuming input unless their first byte
//! matches, so dispatching on that byte reproduces the backtracking, and
//! any failure after it is final.

use nomi_base::{Compat, utf8};

use super::cursor::{Cursor, Position, Region};
use crate::scan::Scan;

/// A parsed value and the region upstream records for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value<'a> {
    pub region: Region,
    pub kind: Kind<'a>,
}

/// What a value is, and what it holds.
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
        Kind::String(_) | Kind::Int(_) | Kind::True | Kind::False | Kind::Null => {}
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
    pub(crate) fn region(&self) -> Region {
        Region {
            start: self.start.position(),
            end: self.start.slide(self.bytes.len()).position(),
        }
    }
}

/// Why a file isn't JSON as upstream parses it, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub at: Position,
}

impl ParseError {
    fn at(cur: Cursor, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            at: cur.position(),
        }
    }
}

/// The parse errors upstream tells apart (`Json.Decode.ParseError`).
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

/// What is wrong inside a string (`Json.Decode.StringProblem`).
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
        scan: Scan::new(src, Cursor::start(compat), compat),
    };
    p.spaces();
    let value = p.value()?;
    p.spaces();
    p.scan
        .finish()
        .map_err(|at| ParseError::at(at, ParseErrorKind::BadEnd))?;
    Ok(value)
}

struct Parser<'a> {
    scan: Scan<'a>,
}

type Parsed<T> = Result<T, ParseError>;

impl<'a> Parser<'a> {
    fn error(&self, kind: ParseErrorKind) -> ParseError {
        ParseError::at(self.scan.cursor(), kind)
    }

    /// `word1`: one expected byte, or `kind` here.
    fn byte(&mut self, byte: u8, kind: ParseErrorKind) -> Parsed<()> {
        self.scan.byte(byte).map_err(|at| ParseError::at(at, kind))
    }

    fn spaces(&mut self) {
        while let Some(byte) = self.scan.peek() {
            match byte {
                b' ' | b'\t' => self.scan.advance(1),
                b'\n' => self.scan.newline(),
                b'\r' => self.scan.skip(1, 0),
                _ => return,
            }
        }
    }

    fn value(&mut self) -> Parsed<Value<'a>> {
        nomi_base::grow(|| {
            let start = self.scan.cursor();
            let kind = match self.scan.peek() {
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
                        return Err(ParseError::at(start, ParseErrorKind::Start));
                    }
                }
            };
            Ok(Value {
                region: Region {
                    start: start.position(),
                    end: self.scan.cursor().position(),
                },
                kind,
            })
        })
    }

    fn object(&mut self) -> Parsed<Kind<'a>> {
        self.byte(b'{', ParseErrorKind::Start)?;
        self.spaces();
        let first = match self.scan.peek() {
            Some(b'"') => self.field()?,
            Some(b'}') => {
                self.scan.advance(1);
                return Ok(Kind::Object(Vec::new()));
            }
            _ => return Err(self.error(ParseErrorKind::ObjectField)),
        };
        let fields = self.rest(first, b'}', ParseErrorKind::ObjectEnd, Self::field)?;
        Ok(Kind::Object(fields))
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
        if self.scan.peek() == Some(b']') {
            self.scan.advance(1);
            return Ok(Kind::Array(Vec::new()));
        }
        let first = self.value()?;
        let entries = self.rest(first, b']', ParseErrorKind::ArrayEnd, Self::value)?;
        Ok(Kind::Array(entries))
    }

    /// The entries of an object or array from its first on: each later
    /// one after a comma, up to `close`, or `end` where neither follows.
    fn rest<T>(
        &mut self,
        first: T,
        close: u8,
        end: ParseErrorKind,
        mut entry: impl FnMut(&mut Self) -> Parsed<T>,
    ) -> Parsed<Vec<T>> {
        let mut entries = vec![first];
        loop {
            self.spaces();
            match self.scan.peek() {
                Some(b',') => {
                    self.scan.advance(1);
                    self.spaces();
                    entries.push(entry(self)?);
                }
                Some(byte) if byte == close => {
                    self.scan.advance(1);
                    return Ok(entries);
                }
                _ => return Err(self.error(end)),
            }
        }
    }

    /// `pString`: `start` if no string starts here, else the string or
    /// the problem in it, where the problem is.
    fn string(&mut self, start: ParseErrorKind) -> Parsed<Snippet<'a>> {
        self.byte(b'"', start)?;
        let bytes = self.scan.rest();
        let (first, first_cur) = (self.scan.pos(), self.scan.cursor());
        loop {
            let cur = self.scan.cursor();
            let Some(byte) = self.scan.peek() else {
                return Err(string_problem(cur, StringProblem::BadStringEnd));
            };
            match byte {
                b'"' => {
                    let snippet = Snippet {
                        bytes: &bytes[..self.scan.pos() - first],
                        start: first_cur,
                    };
                    self.scan.advance(1);
                    return Ok(snippet);
                }
                b'\n' => return Err(string_problem(cur, StringProblem::BadStringEnd)),
                b'\\' => {
                    let width = self.escape()?;
                    self.scan.advance(width);
                }
                0..0x20 => return Err(string_problem(cur, StringProblem::BadStringControlChar)),
                _ => {
                    // 0.19.1 trusts the lead byte and may step past the end,
                    // which then ends the string.
                    let width = self.character()?;
                    self.scan.skip(width, 1);
                }
            }
        }
    }

    /// The width of the escape here, each byte a column.
    fn escape(&self) -> Parsed<usize> {
        let (rest, cur) = (self.scan.rest(), self.scan.cursor());
        match rest.get(1) {
            None => {
                let at = match self.scan.compat() {
                    Compat::V0_19_1 => cur.down(),
                    Compat::V0_19_2 | Compat::V0_19_3 => cur.slide(1),
                };
                Err(string_problem(at, StringProblem::BadStringEnd))
            }
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => Ok(2),
            Some(b'u') => {
                let hex = rest.get(2..6);
                if hex.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) {
                    Ok(6)
                } else {
                    Err(string_problem(cur, StringProblem::BadStringEscapeHex))
                }
            }
            Some(_) => Err(string_problem(cur, StringProblem::BadStringEscapeChar)),
        }
    }

    /// The width in bytes of the character here, as `compat`'s elm steps
    /// over it.
    fn character(&self) -> Parsed<usize> {
        let (rest, cur) = (self.scan.rest(), self.scan.cursor());
        match self.scan.compat() {
            Compat::V0_19_1 => {
                char_width(rest[0]).ok_or_else(|| ParseError::at(cur, ParseErrorKind::Crash))
            }
            Compat::V0_19_2 | Compat::V0_19_3 => {
                utf8_width(rest).ok_or_else(|| string_problem(cur, StringProblem::BadStringNotUtf8))
            }
        }
    }

    /// `pInt`, called on a digit.
    fn int(&mut self) -> Parsed<Kind<'a>> {
        let (rest, cur) = (self.scan.rest(), self.scan.cursor());
        if rest[0] == b'0' {
            return match rest.get(1) {
                Some(b'0'..=b'9') => {
                    Err(ParseError::at(cur.slide(1), ParseErrorKind::NoLeadingZeros))
                }
                Some(b'.') => Err(ParseError::at(cur.slide(1), ParseErrorKind::NoFloats)),
                _ => {
                    self.scan.advance(1);
                    Ok(Kind::Int(0))
                }
            };
        }
        let mut n = 0i64;
        let mut digits = 0;
        for d in rest.iter().take_while(|b| b.is_ascii_digit()) {
            n = n.wrapping_mul(10).wrapping_add(i64::from(d - b'0'));
            digits += 1;
        }
        if let Some(b'.' | b'e' | b'E') = rest.get(digits) {
            return Err(ParseError::at(cur.slide(digits), ParseErrorKind::NoFloats));
        }
        self.scan.advance(digits);
        Ok(Kind::Int(n))
    }

    /// The keyword `word`, not followed by what could continue a name.
    fn keyword(&mut self, word: &[u8]) -> bool {
        let rest = self.scan.rest();
        if !rest.starts_with(word) || utf8::inner_width(&rest[word.len()..], self.scan.compat()) > 0
        {
            return false;
        }
        self.scan.advance(word.len());
        true
    }
}

fn string_problem(cur: Cursor, problem: StringProblem) -> ParseError {
    ParseError::at(cur, ParseErrorKind::String(problem))
}

/// 0.19.1's `getCharWidth`: the width a lead byte announces, or `None`
/// where elm crashes.
fn char_width(lead: u8) -> Option<usize> {
    match lead {
        0x80..0xC0 | 0xF8.. => None,
        lead => Some(utf8::char_width(lead)),
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
