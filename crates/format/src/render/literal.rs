//! Literals, spelled anew from their values; upstream's `formatLiteral`
//! and `formatString`.
//!
//! Numbers print as GHC's `printf` prints them, and strings escape what
//! GHC's `isPrint` and `isSpace` reject.

use std::fmt;

use nomi_base::ghc::{is_print, is_space};
use num_bigint::BigUint;

use super::block::{Block, Line};
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

/// The digits of `f` and the exponent putting the point before them:
/// `f` is `0.d₁d₂… × 10^e`. GHC's `floatToDigits 10` for a finite,
/// non-negative `f`: Burger and Dybvig's free-format algorithm, but with
/// both bounds exclusive, so where the shortest digits lie exactly halfway
/// to a neighbour they are passed over (`1.0e23` prints as
/// `9.999999999999999e22`).
fn digits(f: f64) -> (Vec<u8>, i32) {
    if f == 0.0 {
        return (vec![0], 0);
    }
    let (scaled, k) = scaled(f);
    (generate(scaled), k)
}

/// `floatToDigits`'s state: `r / s` is what is left of the number to
/// spell, and `m_up` and `m_dn` how far its neighbours lie above and
/// below, in the same units.
struct Scaled {
    r: BigUint,
    s: BigUint,
    m_up: BigUint,
    m_dn: BigUint,
}

/// The state for `f`, scaled so that `r / s` is below 1, and the power of
/// ten it was scaled by.
fn scaled(f: f64) -> (Scaled, i32) {
    const P: i32 = 53;
    const MIN_EXP: i32 = -1021 - P;
    // `decodeFloat`, with denormals at `MIN_EXP` as GHC adjusts them.
    let bits = f.to_bits();
    let biased = ((bits >> 52) & 0x7FF) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (m, e) = if biased == 0 {
        (fraction, MIN_EXP)
    } else {
        (fraction | 1 << 52, biased - 1075)
    };
    // `lx` is from GHC's unadjusted, normalized `decodeFloat`.
    let lx = P - 1 + e + (63 - m.leading_zeros() as i32 - 52);
    let at_power = m == 1 << 52;
    let pow = |base: u32, n: i32| BigUint::from(base).pow(n.unsigned_abs());
    let f = BigUint::from(m);
    let (r, s, m_up, m_dn) = if e >= 0 {
        let be = pow(2, e);
        if at_power {
            (&f * &be * 4u32, BigUint::from(4u32), &be * 2u32, be)
        } else {
            (&f * &be * 2u32, BigUint::from(2u32), be.clone(), be)
        }
    } else if e > MIN_EXP && at_power {
        (
            &f * 4u32,
            pow(2, -e + 1) * 2u32,
            BigUint::from(2u32),
            BigUint::from(1u32),
        )
    } else {
        (
            &f * 2u32,
            pow(2, -e) * 2u32,
            BigUint::from(1u32),
            BigUint::from(1u32),
        )
    };
    // `logBase 10 2` is slightly larger than 8651/28738.
    let k1 = lx * 8651 / 28738;
    let mut k = if lx >= 0 { k1 + 1 } else { k1 };
    loop {
        let fits = if k >= 0 {
            &r + &m_up <= pow(10, k) * &s
        } else {
            pow(10, -k) * (&r + &m_up) <= s
        };
        if fits {
            break;
        }
        k += 1;
    }
    let scaled = if k >= 0 {
        Scaled {
            r,
            s: s * pow(10, k),
            m_up,
            m_dn,
        }
    } else {
        let bk = pow(10, -k);
        Scaled {
            r: r * &bk,
            s,
            m_up: m_up * &bk,
            m_dn: m_dn * bk,
        }
    };
    (scaled, k)
}

/// The digits `state` spells, up to where they tell the number from its
/// neighbours.
fn generate(state: Scaled) -> Vec<u8> {
    let Scaled {
        mut r,
        s,
        mut m_up,
        mut m_dn,
    } = state;
    let mut digits = Vec::new();
    loop {
        let scaled = r * 10u32;
        let (d, rest) = (&scaled / &s, &scaled % &s);
        let d = u8::try_from(d).expect("a digit");
        m_up *= 10u32;
        m_dn *= 10u32;
        match (rest < m_dn, &rest + &m_up > s) {
            (false, false) => {
                digits.push(d);
                r = rest;
            }
            (true, false) => break digits.push(d),
            (false, true) => break digits.push(d + 1),
            (true, true) => break digits.push(if &rest * 2u32 < s { d } else { d + 1 }),
        }
    }
    digits
}

/// Digits as their characters.
fn digit_string(digits: &[u8]) -> String {
    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// Upstream's `printf "%f"`, which is GHC's `showFFloat Nothing`.
fn fixed(f: f64) -> String {
    // `1` and 309 zeros, which `0` fills the empty string out to.
    signed(f, format_args!("1{:0<309}.0", ""), |f| {
        let (digits, e) = digits(f);
        let digits = digit_string(&digits);
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
    signed(f, format_args!("1.0e309"), |f| {
        let (digits, e) = digits(f);
        let e = e - 1;
        match &digits[..] {
            [0] => "0.0e0".to_owned(),
            [d] => format!("{d}.0e{e}"),
            [d, rest @ ..] => format!("{d}.{}e{e}", digit_string(rest)),
            [] => unreachable!("a number has digits"),
        }
    })
}

/// `show` around a magnitude's spelling: the sign, and GHC's name for
/// what has no digits. Upstream spells an infinite magnitude, from a
/// literal too large for a double, `Infinity`, a name Elm doesn't
/// define; nomi spells it `infinite`, the least power of ten that reads
/// back as infinite (`DECISIONS.md`).
fn signed(f: f64, infinite: fmt::Arguments<'_>, magnitude: impl FnOnce(f64) -> String) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if f.is_infinite() {
        return format!("{sign}{infinite}");
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

#[cfg(test)]
mod tests;
