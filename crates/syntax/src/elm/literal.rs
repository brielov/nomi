//! `Parse/Variable.hs`, `Parse/Keyword.hs`, `Parse/Symbol.hs`,
//! `Parse/Number.hs`, `Parse/String.hs` and `Parse/Shader.hs`: the
//! parsers that read bytes rather than other parsers.

use nomi_base::{Compat, ghc};

use super::engine::P;
use super::error::{self as E, BadOperator, Cursor, Name};
use crate::glsl::{self, Rejection};

// VARIABLES

/// What `chomp` in `AST/Prim/Variable.hs` takes of a character class: an
/// ASCII byte by `ascii`, anything else decoded and tested by `unicode`.
/// The width it takes, if any.
fn class_width(p: &P<'_>, pos: usize, ascii: fn(u8) -> bool, unicode: fn(char) -> bool) -> usize {
    let Some(b) = p.byte(pos) else {
        return 0;
    };
    if ascii(b) {
        return 1;
    }
    let width = match b {
        0x00..0xC0 | 0xF8.. => return 0,
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    };
    let Some(bytes) = p.byte(pos + width - 1).map(|_| p.bytes(pos, pos + width)) else {
        return 0;
    };
    let c = if p.v1() {
        // 0.19.1 decodes without checking the continuation bytes.
        let lead = u32::from(b) & (0x7F >> width);
        let code = bytes[1..]
            .iter()
            .fold(i64::from(lead), |acc, &w| (acc << 6) + i64::from(w) - 0x80);
        u32::try_from(code).ok().and_then(char::from_u32)
    } else {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.chars().next())
    };
    match c {
        Some(c) if unicode(c) => width,
        _ => 0,
    }
}

fn upper_width(p: &P<'_>, pos: usize) -> usize {
    class_width(p, pos, |b| b.is_ascii_uppercase(), ghc::is_upper)
}

fn lower_width(p: &P<'_>, pos: usize) -> usize {
    class_width(p, pos, |b| b.is_ascii_lowercase(), ghc::is_lower)
}

pub(super) fn inner_width(p: &P<'_>, pos: usize) -> usize {
    class_width(
        p,
        pos,
        |b| b.is_ascii_alphanumeric() || b == b'_',
        ghc::is_alpha,
    )
}

fn chomp_inners(p: &P<'_>, mut pos: usize, mut cur: Cursor) -> (usize, Cursor) {
    loop {
        let width = inner_width(p, pos);
        if width == 0 {
            return (pos, cur);
        }
        pos += width;
        cur = p.slide(cur, 1);
    }
}

fn chomp_upper_var(p: &P<'_>, pos: usize, cur: Cursor) -> (usize, Cursor) {
    match upper_width(p, pos) {
        0 => (pos, cur),
        width => chomp_inners(p, pos + width, p.slide(cur, 1)),
    }
}

fn chomp_lower_var(p: &P<'_>, pos: usize, cur: Cursor) -> (usize, Cursor) {
    match lower_width(p, pos) {
        0 => (pos, cur),
        width => chomp_inners(p, pos + width, p.slide(cur, 1)),
    }
}

const KEYWORDS: [&[u8]; 14] = [
    b"as",
    b"if",
    b"in",
    b"of",
    b"let",
    b"case",
    b"else",
    b"port",
    b"then",
    b"type",
    b"where",
    b"import",
    b"module",
    b"exposing",
];

fn is_keyword(name: &[u8]) -> bool {
    KEYWORDS.contains(&name)
}

fn name(p: &P<'_>, from: usize, to: usize) -> Name {
    String::from_utf8_lossy(p.bytes(from, to)).into()
}

pub(super) fn is_dot(p: &P<'_>, pos: usize) -> bool {
    p.is_word(pos, b'.')
}

/// `Variable.lower`.
pub(super) fn lower<X>(p: &mut P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<Name, X> {
    let (pos, cur) = chomp_lower_var(p, p.pos, p.cur);
    if pos == p.pos || is_keyword(p.bytes(p.pos, pos)) {
        return Err(to_error(p.cur));
    }
    let name = name(p, p.pos, pos);
    p.ok_at(pos, cur);
    Ok(name)
}

/// `Variable.upper`.
pub(super) fn upper<X>(p: &mut P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<Name, X> {
    let (pos, cur) = chomp_upper_var(p, p.pos, p.cur);
    if pos == p.pos {
        return Err(to_error(p.cur));
    }
    let name = name(p, p.pos, pos);
    p.ok_at(pos, cur);
    Ok(name)
}

/// `Variable.moduleName`.
pub(super) fn module_name<X>(p: &mut P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<(), X> {
    let (mut pos, mut cur) = chomp_upper_var(p, p.pos, p.cur);
    if pos == p.pos {
        return Err(to_error(p.cur));
    }
    while is_dot(p, pos) {
        let (new_pos, new_cur) = chomp_upper_var(p, pos + 1, p.slide(cur, 1));
        if new_pos == pos + 1 {
            return p.cerr(to_error(new_cur));
        }
        (pos, cur) = (new_pos, new_cur);
    }
    p.ok_at(pos, cur);
    Ok(())
}

/// `Variable.foreignUpper`.
pub(super) fn foreign_upper<X>(p: &mut P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<(), X> {
    let (mut pos, mut cur) = (p.pos, p.cur);
    let (start, end, end_cur) = loop {
        let (new_pos, new_cur) = chomp_upper_var(p, pos, cur);
        if new_pos == pos {
            break (pos, pos, cur);
        }
        if !is_dot(p, new_pos) {
            break (pos, new_pos, new_cur);
        }
        (pos, cur) = (new_pos + 1, p.slide(new_cur, 1));
    };
    if start == end {
        return Err(to_error(end_cur));
    }
    p.ok_at(end, end_cur);
    Ok(())
}

/// `Variable.foreignAlpha`.
pub(super) fn foreign_alpha<X>(p: &mut P<'_>, to_error: impl FnOnce(Cursor) -> X) -> Result<(), X> {
    let (lower_pos, lower_cur) = chomp_lower_var(p, p.pos, p.cur);
    if lower_pos > p.pos {
        if is_keyword(p.bytes(p.pos, lower_pos)) {
            return Err(to_error(p.cur));
        }
        p.ok_at(lower_pos, lower_cur);
        return Ok(());
    }
    let (mut prev_end, mut cur) = chomp_upper_var(p, p.pos, p.cur);
    if prev_end == p.pos {
        return Err(to_error(p.cur));
    }
    while is_dot(p, prev_end) {
        let pos = prev_end + 1;
        let (lower_pos, lower_cur) = chomp_lower_var(p, pos, p.slide(cur, 1));
        if lower_pos > pos {
            p.ok_at(lower_pos, lower_cur);
            return Ok(());
        }
        let (upper_pos, upper_cur) = chomp_upper_var(p, pos, p.slide(cur, 1));
        if upper_pos == pos {
            // Before 0.19.3, `Variable.foreignAlphaHelp` ends a trailing
            // dot on an empty name and rejects it; 0.19.3 keeps the names
            // before the dot.
            if p.compat < Compat::V0_19_3 {
                return Err(to_error(p.slide(cur, 1)));
            }
            break;
        }
        (prev_end, cur) = (upper_pos, upper_cur);
    }
    p.ok_at(prev_end, cur);
    Ok(())
}

// KEYWORDS

/// A `Parse.Keyword` parser: `word`, not followed by a character a name
/// could go on with.
pub(super) fn keyword<X>(
    p: &mut P<'_>,
    word: &[u8],
    to_error: impl FnOnce(Cursor) -> X,
) -> Result<(), X> {
    let end = p.pos + word.len();
    if end <= p.len() && p.bytes(p.pos, end) == word && inner_width(p, end) == 0 {
        p.ok_at(end, p.slide(p.cur, word.len() as u32));
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

// SYMBOLS

pub(super) fn is_symbol(b: u8) -> bool {
    b"+-/*=.<>:&|^?%!".contains(&b)
}

/// `Symbol.operator`.
pub(super) fn operator<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Cursor) -> X,
    to_error: impl FnOnce(BadOperator, Cursor) -> X,
) -> Result<Name, X> {
    let Some(w) = p.peek(0).filter(|&w| is_symbol(w)) else {
        return Err(to_expectation(p.cur));
    };
    let end = (p.pos + 1..p.len())
        .find(|&i| !is_symbol(p.byte(i).unwrap()))
        .unwrap_or(p.len());
    let width = end - p.pos;
    if width == 1 {
        let bad = match w {
            b'.' => return Err(to_error(BadOperator::BadDot, p.cur)),
            b'|' => Some(BadOperator::BadPipe),
            b'=' => Some(BadOperator::BadEquals),
            b':' => Some(BadOperator::BadHasType),
            _ => None,
        };
        if let Some(bad) = bad {
            let x = to_error(bad, p.cur);
            return p.cerr(x);
        }
    }
    if p.bytes(p.pos, end) == b"->" {
        let x = to_error(BadOperator::BadArrow, p.cur);
        return p.cerr(x);
    }
    let op = name(p, p.pos, end);
    p.ok_at(end, p.slide(p.cur, width as u32));
    Ok(op)
}

// NUMBERS

/// What `Number.number` read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Number {
    Int,
    Float,
}

enum Outcome {
    Err(usize, E::Number),
    Int(usize),
    Float(usize),
}

/// `Number.number`.
pub(super) fn number<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Cursor) -> X,
    to_error: impl FnOnce(E::Number, Cursor) -> X,
) -> Result<Number, X> {
    let Some(w) = p.peek(0).filter(u8::is_ascii_digit) else {
        return Err(to_expectation(p.cur));
    };
    let start = p.pos;
    let outcome = if w == b'0' {
        chomp_zero(p, start + 1)
    } else {
        chomp_int(p, start, start + 1)
    };
    let at = |p: &P<'_>, pos: usize| p.slide(p.cur, (pos - start) as u32);
    match outcome {
        Outcome::Err(pos, problem) => {
            let x = to_error(problem, at(p, pos));
            p.cerr(x)
        }
        Outcome::Int(pos) => {
            p.ok_at(pos, at(p, pos));
            Ok(Number::Int)
        }
        Outcome::Float(pos) => {
            p.ok_at(pos, at(p, pos));
            Ok(Number::Float)
        }
    }
}

fn bad_end(p: &P<'_>, pos: usize) -> bool {
    inner_width(p, pos) > 0
}

fn chomp_int(p: &P<'_>, start: usize, mut pos: usize) -> Outcome {
    loop {
        match p.byte(pos) {
            None => return Outcome::Int(pos),
            Some(b'0'..=b'9') => pos += 1,
            Some(b'.') => {
                let digits = p.bytes(start, pos);
                return chomp_fraction(p, pos, || int_text(p, digits));
            }
            Some(b'e' | b'E') => return chomp_exponent(p, pos + 1),
            Some(_) if bad_end(p, pos) => return Outcome::Err(pos, E::Number::NumberEnd),
            Some(_) => return Outcome::Int(pos),
        }
    }
}

/// The integer before a dot as upstream holds it: 0.19.1 in a wrapping
/// 64-bit `Int`, later releases in an `Integer`.
fn int_text(p: &P<'_>, digits: &[u8]) -> Name {
    if p.v1() {
        let n = digits.iter().fold(0i64, |n, &d| {
            n.wrapping_mul(10).wrapping_add(i64::from(d - b'0'))
        });
        n.to_string().into()
    } else {
        String::from_utf8_lossy(digits).into()
    }
}

fn chomp_fraction(p: &P<'_>, pos: usize, n: impl FnOnce() -> Name) -> Outcome {
    if !p.byte(pos + 1).is_some_and(|b| b.is_ascii_digit()) {
        return Outcome::Err(pos, E::Number::NumberDot(n()));
    }
    let mut pos = pos + 2;
    loop {
        match p.byte(pos) {
            None => return Outcome::Float(pos),
            Some(b'0'..=b'9') => pos += 1,
            Some(b'e' | b'E') => return chomp_exponent(p, pos + 1),
            Some(_) if bad_end(p, pos) => return Outcome::Err(pos, E::Number::NumberEnd),
            Some(_) => return Outcome::Float(pos),
        }
    }
}

fn chomp_exponent(p: &P<'_>, pos: usize) -> Outcome {
    let digits = match p.byte(pos) {
        Some(b'0'..=b'9') => pos + 1,
        Some(b'+' | b'-') if p.byte(pos + 1).is_some_and(|b| b.is_ascii_digit()) => pos + 2,
        _ => return Outcome::Err(pos, E::Number::NumberEnd),
    };
    let end = (digits..p.len())
        .find(|&i| !p.byte(i).unwrap().is_ascii_digit())
        .unwrap_or(p.len());
    Outcome::Float(end)
}

fn chomp_zero(p: &P<'_>, pos: usize) -> Outcome {
    match p.byte(pos) {
        None => Outcome::Int(pos),
        Some(b'x') => match chomp_hex(p, pos + 1) {
            (new_pos, Hex::Bad | Hex::None) => Outcome::Err(new_pos, E::Number::NumberHexDigit),
            (new_pos, Hex::Code(_)) => Outcome::Int(new_pos),
        },
        Some(b'.') => chomp_fraction(p, pos, || "0".into()),
        Some(b'0'..=b'9') => Outcome::Err(pos, E::Number::NumberNoLeadingZero),
        Some(_) if bad_end(p, pos) => Outcome::Err(pos, E::Number::NumberEnd),
        Some(_) => Outcome::Int(pos),
    }
}

/// What `Number.chompHex` found: no digits (`-1`), digits running into
/// a letter (`-2`), or a value. 0.19.1's 64-bit `Int` may wrap negative,
/// which upstream takes for `-2`, or for no digits at exactly `-1`.
#[derive(Clone, Copy)]
pub(super) enum Hex {
    None,
    Bad,
    /// The value, saturated: the escapes that read one only compare it.
    Code(u64),
}

pub(super) fn chomp_hex(p: &P<'_>, mut pos: usize) -> (usize, Hex) {
    let mut answer = Hex::None;
    // 0.19.1's wrapping `Int`, or later releases' `Integer` saturated.
    let mut acc: i64 = 0;
    let mut big: u64 = 0;
    while let Some(w) = p.byte(pos) {
        let digit = match w {
            b'0'..=b'9' => w - b'0',
            b'a'..=b'f' => w - b'a' + 10,
            b'A'..=b'F' => w - b'A' + 10,
            _ if bad_end(p, pos) => return (pos, Hex::Bad),
            _ => return (pos, answer),
        };
        if p.v1() {
            acc = acc.wrapping_mul(16).wrapping_add(i64::from(digit));
            match acc {
                -1 => return (pos, answer),
                ..0 => return (pos, Hex::Bad),
                _ => answer = Hex::Code(acc as u64),
            }
        } else {
            big = big.saturating_mul(16).saturating_add(u64::from(digit));
            answer = Hex::Code(big);
        }
        pos += 1;
    }
    (pos, answer)
}

// CHARACTERS AND STRINGS

enum Escape {
    /// The escape's width.
    Ok(usize),
    EndOfFile,
    Problem(E::Escape),
}

/// `eatEscape`, after the backslash at `pos - 1`.
fn eat_escape(p: &P<'_>, pos: usize) -> Escape {
    match p.byte(pos) {
        None => Escape::EndOfFile,
        Some(b'n' | b'r' | b't' | b'"' | b'\'' | b'\\') => Escape::Ok(2),
        Some(b'u') => eat_unicode(p, pos + 1),
        Some(_) => Escape::Problem(E::Escape::EscapeUnknown),
    }
}

fn eat_unicode(p: &P<'_>, pos: usize) -> Escape {
    if p.byte(pos) != Some(b'{') {
        return Escape::Problem(E::Escape::BadUnicodeFormat(2));
    }
    let digit_pos = pos + 1;
    let (new_pos, code) = chomp_hex(p, digit_pos);
    let num_digits = (new_pos - digit_pos) as u32;
    let width = (new_pos - pos) as u32;
    let width = |extra: u32| p.word(extra + width);
    if p.byte(new_pos) != Some(b'}') {
        return Escape::Problem(E::Escape::BadUnicodeFormat(width(2)));
    }
    let code = match code {
        Hex::Code(code) if code <= 0x10FFFF => code as u32,
        _ => return Escape::Problem(E::Escape::BadUnicodeCode(width(3))),
    };
    if !(4..=6).contains(&num_digits) {
        return Escape::Problem(E::Escape::BadUnicodeLength(width(3), num_digits, code));
    }
    Escape::Ok(num_digits as usize + 4)
}

/// What [`utf8_char`] found at a byte past ASCII.
enum Utf8 {
    Char(usize),
    Invalid,
    Truncated,
}

/// The UTF-8 character starting at `pos`, which 0.19.1 takes on trust.
fn utf8_char(p: &P<'_>, pos: usize) -> Utf8 {
    if p.v1() {
        return Utf8::Char(p.skip_utf8(pos));
    }
    let window = p.bytes(pos, pos + 4);
    match std::str::from_utf8(window) {
        Ok(s) => Utf8::Char(s.chars().next().map_or(1, char::len_utf8)),
        Err(e) if e.valid_up_to() > 0 => Utf8::Char(
            std::str::from_utf8(&window[..e.valid_up_to()])
                .ok()
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8),
        ),
        Err(e) if e.error_len().is_none() => Utf8::Truncated,
        Err(_) => Utf8::Invalid,
    }
}

/// `String.character`.
pub(super) fn character<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Cursor) -> X,
    to_error: impl FnOnce(E::Char, Cursor) -> X,
) -> Result<(), X> {
    if p.peek(0) != Some(b'\'') {
        return Err(to_expectation(p.cur));
    }
    let (mut pos, mut cur) = (p.pos + 1, p.slide(p.cur, 1));
    let mut chars = 0;
    let fail = |p: &mut P<'_>, problem, at| {
        let x = to_error(problem, at);
        p.cerr(x)
    };
    loop {
        match p.byte(pos) {
            None | Some(b'\n') => return fail(p, E::Char::CharEndless, cur),
            Some(b'\'') => {
                pos += 1;
                cur = p.slide(cur, 1);
                break;
            }
            Some(b'\\') => match eat_escape(p, pos + 1) {
                Escape::Ok(width) => {
                    pos += width;
                    cur = p.slide(cur, width as u32);
                    chars += 1;
                }
                Escape::Problem(e) => return fail(p, E::Char::CharEscape(e), cur),
                Escape::EndOfFile => return fail(p, E::Char::CharEndless, cur),
            },
            Some(0x00..0x80) => {
                pos += 1;
                cur = p.slide(cur, 1);
                chars += 1;
            }
            Some(_) => match utf8_char(p, pos) {
                Utf8::Char(width) => {
                    pos += width;
                    cur = p.slide(cur, 1);
                    chars += 1;
                }
                Utf8::Invalid | Utf8::Truncated => return fail(p, E::Char::CharNotUtf8, cur),
            },
        }
    }
    if chars != 1 {
        let width = p.word(cur.col.wrapping_sub(p.cur.col));
        let at = p.cur;
        return fail(p, E::Char::CharNotString(width), at);
    }
    p.ok_at(pos, cur);
    Ok(())
}

/// `String.string`.
pub(super) fn string<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Cursor) -> X,
    to_error: impl FnOnce(E::String, Cursor) -> X,
) -> Result<(), X> {
    if p.peek(0) != Some(b'"') {
        return Err(to_expectation(p.cur));
    }
    let result = if p.peek(1) == Some(b'"') {
        if p.peek(2) == Some(b'"') {
            multi_string(p)
        } else {
            Ok((p.pos + 2, p.slide(p.cur, 2)))
        }
    } else {
        single_string(p, p.pos + 1, p.slide(p.cur, 1))
    };
    match result {
        Ok((pos, cur)) => {
            p.ok_at(pos, cur);
            Ok(())
        }
        Err((problem, at)) => {
            let x = to_error(problem, at);
            p.cerr(x)
        }
    }
}

type Chomped = Result<(usize, Cursor), (E::String, Cursor)>;

fn single_string(p: &P<'_>, mut pos: usize, mut cur: Cursor) -> Chomped {
    use E::String::{StringEndlessSingle as Endless, StringEscape, StringNotUtf8};
    loop {
        match p.byte(pos) {
            None | Some(b'\n') => return Err((Endless, cur)),
            Some(b'"') => return Ok((pos + 1, p.slide(cur, 1))),
            Some(b'\\') => match eat_escape(p, pos + 1) {
                Escape::Ok(width) => {
                    pos += width;
                    cur = p.slide(cur, width as u32);
                }
                Escape::Problem(e) => return Err((StringEscape(e), cur)),
                Escape::EndOfFile => return Err((Endless, p.slide(cur, 1))),
            },
            Some(0x00..0x80) => {
                pos += 1;
                cur = p.slide(cur, 1);
            }
            Some(_) => match utf8_char(p, pos) {
                Utf8::Char(width) => {
                    pos += width;
                    cur = p.slide(cur, 1);
                }
                Utf8::Invalid => return Err((StringNotUtf8, cur)),
                Utf8::Truncated => return Err((Endless, cur)),
            },
        }
    }
}

fn multi_string(p: &P<'_>) -> Chomped {
    use E::String::{StringEndlessMulti as Endless, StringEscape, StringNotUtf8};
    let start = p.cur;
    let (mut pos, mut cur) = (p.pos + 3, p.slide(p.cur, 3));
    loop {
        match p.byte(pos) {
            None => return Err((Endless, start)),
            Some(b'"') if p.byte(pos + 1) == Some(b'"') && p.byte(pos + 2) == Some(b'"') => {
                return Ok((pos + 3, p.slide(cur, 3)));
            }
            Some(b'\n') => {
                pos += 1;
                cur = p.newline(cur);
            }
            Some(b'\r') => pos += 1,
            Some(b'\\') => match eat_escape(p, pos + 1) {
                Escape::Ok(width) => {
                    pos += width;
                    cur = p.slide(cur, width as u32);
                }
                Escape::Problem(e) => return Err((StringEscape(e), cur)),
                Escape::EndOfFile => return Err((Endless, start)),
            },
            Some(0x00..0x80) => {
                pos += 1;
                cur = p.slide(cur, 1);
            }
            Some(_) => match utf8_char(p, pos) {
                Utf8::Char(width) => {
                    pos += width;
                    cur = p.slide(cur, 1);
                }
                Utf8::Invalid => return Err((StringNotUtf8, cur)),
                Utf8::Truncated => return Err((Endless, start)),
            },
        }
    }
}

// SHADERS

/// `Shader.shader`: the block, then its GLSL.
pub(super) fn shader(p: &mut P<'_>) -> Result<(), E::Expr> {
    let start = p.cur;
    if p.bytes(p.pos, p.pos + 6) != b"[glsl|" {
        return Err(E::Expr::Start(start));
    }
    let text_start = p.pos + 6;
    let (mut pos, mut cur) = (text_start, p.slide(start, 6));
    loop {
        match p.byte(pos) {
            None => return p.cerr(E::Expr::ShaderEndless(start)),
            Some(b'|') if p.byte(pos + 1) == Some(b']') => break,
            Some(b'\n') => {
                pos += 1;
                cur = p.newline(cur);
            }
            Some(_) => match p.skip_utf8(pos) {
                0 => return p.cerr(E::Expr::ShaderNotUtf8(start)),
                width => {
                    pos += width;
                    cur = p.slide(cur, 1);
                }
            },
        }
    }
    let text = String::from_utf8_lossy(p.bytes(text_start, pos));
    p.ok_at(pos + 2, p.slide(cur, 2));
    let (offset, message) = match glsl::check(&text) {
        Ok(()) => return Ok(()),
        Err(Rejection::At { offset, message }) => (offset, message),
        // Upstream has no depth limit, so no message to quote.
        Err(Rejection::TooDeep(offset)) => (offset, String::new()),
    };
    // Parsec's position: lines from 1, columns from 1 by character, tabs
    // to the next multiple of 8.
    let (mut line, mut col) = (1u32, 1u32);
    for c in text[..offset].chars() {
        match c {
            '\n' => (line, col) = (line + 1, 1),
            '\t' => col = col + 8 - (col - 1) % 8,
            _ => col += 1,
        }
    }
    let at = if p.v1() {
        if line == 1 {
            Cursor {
                row: start.row,
                col: p.word(start.col + 6 + col),
            }
        } else {
            Cursor {
                row: p.word(start.row + line - 1),
                col: p.word(col),
            }
        }
    } else if line == 1 {
        p.slide(p.slide(start, 6), col - 1)
    } else {
        let row_start = Cursor {
            row: start.row + line - 1,
            col: 0,
        };
        p.slide(row_start, col - 1)
    };
    p.cerr(E::Expr::ShaderProblem(message.into(), at))
}
