//! Literals, spelled anew from their values; upstream's `formatLiteral`
//! and `formatString`.
//!
//! Numbers print as GHC's `printf` prints them, and strings escape what
//! GHC's `isPrint` and `isSpace` reject. Plan 2.12 checks the spelling
//! against the oracle across the corpus, and settles the Unicode tables
//! those two predicates read, which this approximates.

use crate::block::{Block, Line};
use crate::tree::{FloatRepr, IntRepr, LiteralValue, StringRepr};

/// Upstream's `formatLiteral`.
pub(super) fn literal(value: &LiteralValue) -> Block {
    let text = match value {
        LiteralValue::Int(i, IntRepr::Decimal) => i.to_string(),
        LiteralValue::Int(i, IntRepr::Hexadecimal) => hexadecimal(*i),
        LiteralValue::Float(f, FloatRepr::Decimal) => fixed(*f),
        LiteralValue::Float(f, FloatRepr::Exponent) => exponent(*f),
        LiteralValue::Char(c) => string(Style::Char, &[*c]),
        LiteralValue::String(s, repr) => string(Style::String(*repr), s),
        LiteralValue::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
    };
    Block::line(Line::text(text))
}

/// At least two digits, and as many more as the next of 4, 8 or 16 that
/// holds the magnitude.
fn hexadecimal(i: i64) -> String {
    let sign = if i < 0 { "-" } else { "" };
    let magnitude = i.unsigned_abs();
    let width = match magnitude {
        0..=0xFF => 2,
        0x100..=0xFFFF => 4,
        0x1_0000..=0xFFFF_FFFF => 8,
        _ => 16,
    };
    format!("{sign}0x{magnitude:0width$X}")
}

/// The shortest digits that read back as `f`, and the exponent putting
/// the point before them: `f` is `0.d₁d₂… × 10^e`. GHC's `floatToDigits`
/// for a finite, non-negative `f`.
fn digits(f: f64) -> (Vec<u8>, i32) {
    if f == 0.0 {
        return (vec![0], 0);
    }
    let scientific = format!("{f:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("`{:e}` has an `e`");
    let exponent: i32 = exponent.parse().expect("`{:e}` has an integer exponent");
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|d| d - b'0')
        .collect();
    (digits, exponent + 1)
}

/// Upstream's `printf "%f"`, which is GHC's `showFFloat Nothing`.
fn fixed(f: f64) -> String {
    signed(f, |f| {
        let (digits, e) = digits(f);
        let digits: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
        if e <= 0 {
            return format!("0.{}{digits}", "0".repeat(e.unsigned_abs() as usize));
        }
        let e = e as usize;
        let (whole, fraction) = if digits.len() > e {
            (digits[..e].to_owned(), digits[e..].to_owned())
        } else {
            (
                format!("{digits}{}", "0".repeat(e - digits.len())),
                String::new(),
            )
        };
        let or_zero = |s: String| if s.is_empty() { "0".to_owned() } else { s };
        format!("{}.{}", or_zero(whole), or_zero(fraction))
    })
}

/// Upstream's `printf "%e"`, which is GHC's `showEFloat Nothing`.
fn exponent(f: f64) -> String {
    signed(f, |f| {
        let (digits, e) = digits(f);
        let e = e - 1;
        match &digits[..] {
            [0] => "0.0e0".to_owned(),
            [d] => format!("{d}.0e{e}"),
            [d, rest @ ..] => {
                let rest: String = rest.iter().map(|d| char::from(b'0' + d)).collect();
                format!("{d}.{rest}e{e}")
            }
            [] => unreachable!("a number has digits"),
        }
    })
}

/// `show` around a magnitude's spelling: the sign, and GHC's names for
/// what has no digits.
fn signed(f: f64, magnitude: impl FnOnce(f64) -> String) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if f.is_infinite() {
        return format!("{sign}Infinity");
    }
    format!("{sign}{}", magnitude(f.abs()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Char,
    String(StringRepr),
}

/// Upstream's `formatString`: the code points between quotes, escaped.
fn string(style: Style, code_points: &[u32]) -> String {
    let quotes = match style {
        Style::Char => "'",
        Style::String(StringRepr::SingleQuoted) => "\"",
        Style::String(StringRepr::TripleQuoted) => "\"\"\"",
    };
    let mut body = String::new();
    for &c in code_points {
        escape(style, c, &mut body);
    }
    if style == Style::String(StringRepr::TripleQuoted) {
        body = escape_multi_quote(&body);
    }
    format!("{quotes}{body}{quotes}")
}

/// Upstream's `fix`, for Elm 0.19.
fn escape(style: Style, c: u32, out: &mut String) {
    let triple = style == Style::String(StringRepr::TripleQuoted);
    let printable = char::from_u32(c).filter(|&ch| is_print(ch));
    match (c, printable) {
        (0x0A, _) if triple => out.push('\n'),
        (0x0A, _) => out.push_str("\\n"),
        (0x09, _) => out.push_str("\\t"),
        (0x5C, _) => out.push_str("\\\\"),
        (0x22, _) if style == Style::String(StringRepr::SingleQuoted) => out.push_str("\\\""),
        (0x27, _) if style == Style::Char => out.push_str("\\'"),
        (_, Some(' ')) => out.push(' '),
        (_, Some(ch)) if !is_space(ch) => out.push(ch),
        _ => out.push_str(&format!("\\u{{{c:04X}}}")),
    }
}

/// Upstream's `escapeMultiQuote`: three or more `"` in a row, and any at
/// the end, as `\"`.
fn escape_multi_quote(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut quotes = 0;
    for c in body.chars() {
        if c == '"' {
            quotes += 1;
            continue;
        }
        out.push_str(&if quotes >= 3 { "\\\"" } else { "\"" }.repeat(quotes));
        quotes = 0;
        out.push(c);
    }
    out.push_str(&"\\\"".repeat(quotes));
    out
}

/// GHC's `isSpace`: the Latin-1 spaces, and the space separators.
fn is_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'..='\r' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// GHC's `isPrint`: not a control, format, surrogate, private-use or
/// unassigned character, nor a line or paragraph separator. Unassigned
/// code points read as printable until plan 2.12 brings GHC's table.
fn is_print(c: char) -> bool {
    !(c.is_control()
        || matches!(
            c,
            '\u{AD}'
                | '\u{600}'..='\u{605}'
                | '\u{61C}'
                | '\u{6DD}'
                | '\u{70F}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E000}'..='\u{F8FF}'
                | '\u{F0000}'..='\u{FFFFD}'
                | '\u{100000}'..='\u{10FFFD}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        ))
}

#[cfg(test)]
mod tests;
