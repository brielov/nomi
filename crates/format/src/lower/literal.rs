//! Literals, decoded as elm-format's parser decodes them.
//!
//! elm-format writes each escape's code point out as UTF-8 bytes, cut to
//! 8 bits each, and reads the bytes back as characters without checks.
//! Decoding does the same, so `"\u{D835}\u{DC00}"` stays two surrogates
//! and the `\x` escapes elm-format still reads come out as it reads them.

use nomi_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

use super::{VALID, significant};
use crate::ghc::is_alpha;
use crate::tree::{FloatRepr, IntRepr, LiteralValue, StringRepr};

/// The literal `node` holds: a number with an optional `-` before it, a
/// character or a string.
pub(super) fn literal(node: &SyntaxNode) -> LiteralValue {
    let mut tokens = significant(node);
    let mut token = tokens.next().expect(VALID);
    if token.kind() == SyntaxKind::Minus {
        token = tokens.next().expect(VALID);
        return negative(unsigned(&token));
    }
    unsigned(&token)
}

/// Whether `node` is a number, which a `-` touching it negates.
pub(super) fn is_number(node: &SyntaxNode) -> bool {
    let token = significant(node).next().expect(VALID);
    matches!(token.kind(), SyntaxKind::Int | SyntaxKind::Float)
}

/// A number, negated; upstream's `num`, which reads the `-` itself.
pub(super) fn negative(value: LiteralValue) -> LiteralValue {
    match value {
        LiteralValue::Int(value, repr) => LiteralValue::Int(value.wrapping_neg(), repr),
        LiteralValue::Float(value, repr) => LiteralValue::Float(-value, repr),
        other => panic!("{VALID}: a number, not {other:?}"),
    }
}

fn unsigned(token: &SyntaxToken) -> LiteralValue {
    let text = token.text();
    match token.kind() {
        SyntaxKind::Int => {
            let (value, repr) = int(text);
            LiteralValue::Int(value, repr)
        }
        SyntaxKind::Float => {
            let value: f64 = text.parse().expect(VALID);
            let repr = if text.contains(['e', 'E']) {
                FloatRepr::Exponent
            } else {
                FloatRepr::Decimal
            };
            LiteralValue::Float(value, repr)
        }
        SyntaxKind::Char => match decode(&text[1..text.len() - 1])[..] {
            [c] => LiteralValue::Char(c),
            _ => panic!("{VALID}: one character in {text}"),
        },
        SyntaxKind::String => {
            LiteralValue::String(decode(&text[1..text.len() - 1]), StringRepr::SingleQuoted)
        }
        SyntaxKind::MultilineString => {
            LiteralValue::String(decode(&text[3..text.len() - 3]), StringRepr::TripleQuoted)
        }
        kind => panic!("{VALID}: a literal, not {kind:?}"),
    }
}

/// An integer's value, wrapping as upstream's 64-bit chompers do.
fn int(text: &str) -> (i64, IntRepr) {
    match text.strip_prefix("0x") {
        Some(hex) => (digits(hex, 16), IntRepr::Hexadecimal),
        None => (digits(text, 10), IntRepr::Decimal),
    }
}

fn digits(text: &str, radix: u32) -> i64 {
    text.chars()
        .map(|c| i64::from(c.to_digit(radix).expect(VALID)))
        .fold(0, |n, d| n.wrapping_mul(i64::from(radix)).wrapping_add(d))
}

/// The characters of a literal's body; upstream's `fromChunks` and
/// `toChars`. A line break in the source reads as `\n`.
fn decode(body: &str) -> Vec<u32> {
    let mut bytes = Vec::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find(['\\', '\r']) {
        bytes.extend_from_slice(&rest.as_bytes()[..i]);
        let after = &rest[i + 1..];
        rest = if rest.as_bytes()[i] == b'\r' {
            match after.strip_prefix('\n') {
                Some(after) => {
                    bytes.push(b'\n');
                    after
                }
                None => {
                    bytes.push(b'\r');
                    after
                }
            }
        } else {
            escape(after, &mut bytes)
        };
    }
    bytes.extend_from_slice(rest.as_bytes());
    to_chars(&bytes)
}

/// Writes the escape `text` starts with, after its `\`, and returns the
/// text after it.
fn escape<'a>(text: &'a str, bytes: &mut Vec<u8>) -> &'a str {
    let (code, rest) = match text.as_bytes().first().expect(VALID) {
        b'n' => (0x0A, &text[1..]),
        b'r' => (0x0D, &text[1..]),
        b't' => (0x09, &text[1..]),
        &b @ (b'"' | b'\'' | b'\\') => (i64::from(b), &text[1..]),
        b'x' => hex(&text[1..]),
        b'u' => {
            let (code, rest) = hex(text[1..].strip_prefix('{').expect(VALID));
            (code, rest.strip_prefix('}').expect(VALID))
        }
        _ => panic!("{VALID}: a known escape"),
    };
    write_code(code, bytes);
    rest
}

/// Leading hex digits and the text after them; upstream's `chompHex`,
/// which gives -1 for none and -2 when a letter, digit or `_` follows
/// them. Only a `\x` escape can be followed so, and upstream writes the
/// -2 out regardless.
fn hex(text: &str) -> (i64, &str) {
    let end = text
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(text.len());
    let rest = &text[end..];
    match rest.chars().next() {
        Some(c) if c == '_' || c.is_ascii_alphanumeric() || (!c.is_ascii() && is_alpha(c)) => {
            (-2, rest)
        }
        _ if end == 0 => (-1, rest),
        _ => (digits(&text[..end], 16), rest),
    }
}

/// Upstream's `writeCode`: UTF-8 for any integer, each byte cut to 8
/// bits.
fn write_code(c: i64, bytes: &mut Vec<u8>) {
    let byte = |n: i64| n as u8;
    if c <= 0x7F {
        bytes.push(byte(c));
    } else if c <= 0x7FF {
        bytes.extend([byte(0xC0 + (c >> 6)), byte(0x80 + (c & 0x3F))]);
    } else if c <= 0xFFFF {
        bytes.extend([
            byte(0xE0 + (c >> 12)),
            byte(0x80 + ((c >> 6) & 0x3F)),
            byte(0x80 + (c & 0x3F)),
        ]);
    } else {
        bytes.extend([
            byte(0xF0 + (c >> 18)),
            byte(0x80 + ((c >> 12) & 0x3F)),
            byte(0x80 + ((c >> 6) & 0x3F)),
            byte(0x80 + (c & 0x3F)),
        ]);
    }
}

/// Upstream's `toChars`: UTF-8 read by its first byte alone. A sequence
/// cut off by the end, which only a nonsensical escape writes, reads the
/// missing bytes as zero.
fn to_chars(bytes: &[u8]) -> Vec<u32> {
    let mut chars = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let first = bytes[i];
        let (width, lead) = match first {
            0x00..0xC0 => (1, i64::from(first)),
            0xC0..0xE0 => (2, i64::from(first) - 0xC0),
            0xE0..0xF0 => (3, i64::from(first) - 0xE0),
            _ => (4, i64::from(first) - 0xF0),
        };
        let code = (1..width).fold(lead, |code, k| {
            let next = bytes.get(i + k).copied().map_or(0, i64::from);
            (code << 6) + next - 0x80
        });
        chars.push(code as u32);
        i += width;
    }
    chars
}
