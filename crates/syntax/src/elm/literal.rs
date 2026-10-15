//! `Parse/Variable.hs`, `Parse/Keyword.hs`, `Parse/Symbol.hs`,
//! `Parse/Number.hs`, `Parse/String.hs` and `Parse/Shader.hs`: the
//! parsers that read bytes rather than other parsers.

use nomi_base::diagnostic::code::{BINOP_CHARS, Keyword};
use nomi_base::{Compat, utf8};

use super::engine::{Loc, P};
use super::error::{self as E, BadOperator, Name, Position};
use crate::glsl::{self, Rejection};

// VARIABLES

/// The width of the character at `pos` if `class` takes it, or zero.
fn class_width(p: &P<'_>, pos: usize, class: fn(&[u8], Compat) -> usize) -> usize {
    class(p.src().get(pos..).unwrap_or_default(), p.compat)
}

pub(super) fn inner_width(p: &P<'_>, pos: usize) -> usize {
    class_width(p, pos, utf8::inner_width)
}

/// The characters from `at` that go on with a name, and where they end.
pub(super) fn chomp_inners(p: &P<'_>, mut at: Loc) -> Loc {
    loop {
        match inner_width(p, at.pos) {
            0 => return at,
            width => at = at.step(p, width, 1),
        }
    }
}

/// A name from `at` whose first character `first` takes, and where it
/// ends: `at` if there is none.
fn chomp_var(p: &P<'_>, at: Loc, first: fn(&[u8], Compat) -> usize) -> Loc {
    match class_width(p, at.pos, first) {
        0 => at,
        width => chomp_inners(p, at.step(p, width, 1)),
    }
}

fn is_keyword(name: &[u8]) -> bool {
    Keyword::ALL.iter().any(|k| k.as_str().as_bytes() == name)
}

/// The text from `from` to `to`, as a name.
pub(super) fn name(p: &P<'_>, from: usize, to: usize) -> Name {
    String::from_utf8_lossy(p.bytes(from, to)).into()
}

/// `Variable.lower`.
pub(super) fn lower<X>(p: &mut P<'_>, to_error: impl FnOnce(Position) -> X) -> Result<Name, X> {
    let end = chomp_var(p, p.here(), utf8::lower_width);
    if end.pos == p.pos || is_keyword(p.bytes(p.pos, end.pos)) {
        return Err(to_error(p.cur));
    }
    let name = name(p, p.pos, end.pos);
    p.ok_at(end);
    Ok(name)
}

/// `Variable.upper`.
pub(super) fn upper<X>(p: &mut P<'_>, to_error: impl FnOnce(Position) -> X) -> Result<Name, X> {
    let end = chomp_var(p, p.here(), utf8::upper_width);
    if end.pos == p.pos {
        return Err(to_error(p.cur));
    }
    let name = name(p, p.pos, end.pos);
    p.ok_at(end);
    Ok(name)
}

/// `chompMatchingName`: the name a type annotation is for, again, or
/// `to_repeat` where there is no name and `to_mismatch` where there is
/// another.
pub(super) fn matching_name<X>(
    p: &mut P<'_>,
    expected: &Name,
    to_repeat: fn(Position) -> X,
    to_mismatch: fn(Name, Position) -> X,
) -> Result<(), X> {
    let start = p.cur;
    let name = lower(p, to_repeat)?;
    if name == *expected {
        Ok(())
    } else {
        p.cerr(to_mismatch(name, start))
    }
}

/// `Variable.moduleName`.
pub(super) fn module_name<X>(p: &mut P<'_>, to_error: impl FnOnce(Position) -> X) -> Result<(), X> {
    let mut end = chomp_var(p, p.here(), utf8::upper_width);
    if end.pos == p.pos {
        return Err(to_error(p.cur));
    }
    while p.is_word(end.pos, b'.') {
        let dot = end.step(p, 1, 1);
        let next = chomp_var(p, dot, utf8::upper_width);
        if next.pos == dot.pos {
            return p.cerr(to_error(next.cur));
        }
        end = next;
    }
    p.ok_at(end);
    Ok(())
}

/// `Variable.foreignUpper`.
pub(super) fn foreign_upper<X>(
    p: &mut P<'_>,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    let mut at = p.here();
    loop {
        let end = chomp_var(p, at, utf8::upper_width);
        if end.pos == at.pos {
            return Err(to_error(at.cur));
        }
        if !p.is_word(end.pos, b'.') {
            p.ok_at(end);
            return Ok(());
        }
        at = end.step(p, 1, 1);
    }
}

/// `Variable.foreignAlpha`.
pub(super) fn foreign_alpha<X>(
    p: &mut P<'_>,
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    let lower_end = chomp_var(p, p.here(), utf8::lower_width);
    if lower_end.pos > p.pos {
        if is_keyword(p.bytes(p.pos, lower_end.pos)) {
            return Err(to_error(p.cur));
        }
        p.ok_at(lower_end);
        return Ok(());
    }
    let mut end = chomp_var(p, p.here(), utf8::upper_width);
    if end.pos == p.pos {
        return Err(to_error(p.cur));
    }
    while p.is_word(end.pos, b'.') {
        let dot = end.step(p, 1, 1);
        let lower_end = chomp_var(p, dot, utf8::lower_width);
        if lower_end.pos > dot.pos {
            p.ok_at(lower_end);
            return Ok(());
        }
        let upper_end = chomp_var(p, dot, utf8::upper_width);
        if upper_end.pos == dot.pos {
            // Before 0.19.3, `Variable.foreignAlphaHelp` ends a trailing
            // dot on an empty name and rejects it; 0.19.3 keeps the names
            // before the dot.
            if p.compat < Compat::V0_19_3 {
                return Err(to_error(dot.cur));
            }
            break;
        }
        end = upper_end;
    }
    p.ok_at(end);
    Ok(())
}

// KEYWORDS

/// A `Parse.Keyword` parser: `word`, not followed by a character a name
/// could go on with.
pub(super) fn keyword<X>(
    p: &mut P<'_>,
    word: &[u8],
    to_error: impl FnOnce(Position) -> X,
) -> Result<(), X> {
    let end = p.pos + word.len();
    if end <= p.len() && p.bytes(p.pos, end) == word && inner_width(p, end) == 0 {
        p.advance(word.len(), word.len() as u32);
        Ok(())
    } else {
        Err(to_error(p.cur))
    }
}

// SYMBOLS

fn is_symbol(b: u8) -> bool {
    BINOP_CHARS.as_bytes().contains(&b)
}

/// `Symbol.operator`.
pub(super) fn operator<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Position) -> X,
    to_error: impl FnOnce(BadOperator, Position) -> X,
) -> Result<Name, X> {
    let Some(w) = p.peek(0).filter(|&w| is_symbol(w)) else {
        return Err(to_expectation(p.cur));
    };
    let rest = p.pos + 1;
    let end = (p.bytes(rest, p.len()).iter())
        .position(|&b| !is_symbol(b))
        .map_or(p.len(), |i| rest + i);
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
    p.advance(width, width as u32);
    Ok(op)
}

// NUMBERS

/// What `Number.number` read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Number {
    Int,
    Float,
}

/// Where a number ends and what it is, or where it goes wrong and how.
type Scanned = Result<(usize, Number), (usize, E::Number)>;

/// `Number.number`.
pub(super) fn number<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Position) -> X,
    to_error: impl FnOnce(E::Number, Position) -> X,
) -> Result<Number, X> {
    let Some(w) = p.peek(0).filter(u8::is_ascii_digit) else {
        return Err(to_expectation(p.cur));
    };
    let start = p.pos;
    let scanned = if w == b'0' {
        chomp_zero(p, start + 1)
    } else {
        chomp_int(p, start, start + 1)
    };
    match scanned {
        Ok((end, number)) => {
            p.advance(end - start, (end - start) as u32);
            Ok(number)
        }
        Err((at, problem)) => {
            let x = to_error(problem, p.slide(p.cur, (at - start) as u32));
            p.cerr(x)
        }
    }
}

fn bad_end(p: &P<'_>, pos: usize) -> bool {
    inner_width(p, pos) > 0
}

fn chomp_int(p: &P<'_>, start: usize, mut pos: usize) -> Scanned {
    loop {
        match p.byte(pos) {
            Some(b'0'..=b'9') => pos += 1,
            Some(b'.') => {
                let digits = p.bytes(start, pos);
                return chomp_fraction(p, pos, || int_text(p, digits));
            }
            Some(b'e' | b'E') => return chomp_exponent(p, pos + 1),
            Some(_) if bad_end(p, pos) => return Err((pos, E::Number::NumberEnd)),
            _ => return Ok((pos, Number::Int)),
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

fn chomp_fraction(p: &P<'_>, pos: usize, n: impl FnOnce() -> Name) -> Scanned {
    if !p.byte(pos + 1).is_some_and(|b| b.is_ascii_digit()) {
        return Err((pos, E::Number::NumberDot(n())));
    }
    let mut pos = pos + 2;
    loop {
        match p.byte(pos) {
            Some(b'0'..=b'9') => pos += 1,
            Some(b'e' | b'E') => return chomp_exponent(p, pos + 1),
            Some(_) if bad_end(p, pos) => return Err((pos, E::Number::NumberEnd)),
            _ => return Ok((pos, Number::Float)),
        }
    }
}

fn chomp_exponent(p: &P<'_>, pos: usize) -> Scanned {
    let digits = match p.byte(pos) {
        Some(b'0'..=b'9') => pos + 1,
        Some(b'+' | b'-') if p.byte(pos + 1).is_some_and(|b| b.is_ascii_digit()) => pos + 2,
        _ => return Err((pos, E::Number::NumberEnd)),
    };
    let end = (p.bytes(digits, p.len()).iter())
        .position(|b| !b.is_ascii_digit())
        .map_or(p.len(), |i| digits + i);
    Ok((end, Number::Float))
}

fn chomp_zero(p: &P<'_>, pos: usize) -> Scanned {
    match p.byte(pos) {
        Some(b'x') => match chomp_hex(p.src(), pos + 1, p.compat, |at| bad_end(p, at)) {
            (end, Hex::Code(_)) => Ok((end, Number::Int)),
            (end, Hex::Bad | Hex::None) => Err((end, E::Number::NumberHexDigit)),
        },
        Some(b'.') => chomp_fraction(p, pos, || "0".into()),
        Some(b'0'..=b'9') => Err((pos, E::Number::NumberNoLeadingZero)),
        Some(_) if bad_end(p, pos) => Err((pos, E::Number::NumberEnd)),
        _ => Ok((pos, Number::Int)),
    }
}

/// What `Number.chompHex` found: no digits (`-1`), digits running into
/// a letter (`-2`), or a value. 0.19.1's 64-bit `Int` may wrap negative,
/// which upstream takes for `-2`, or for no digits at exactly `-1`.
#[derive(Clone, Copy)]
pub(crate) enum Hex {
    None,
    Bad,
    /// The value, saturated: the escapes that read one only compare it.
    Code(u64),
}

/// `Number.chompHex` over `src` from `pos`, as `compat` reads it: where
/// the digits end, and what they are. `dirty` says whether what is at a
/// position would go on with a name. elm-format reads hexadecimal as
/// 0.19.1 does.
pub(crate) fn chomp_hex(
    src: &[u8],
    mut pos: usize,
    compat: Compat,
    dirty: impl Fn(usize) -> bool,
) -> (usize, Hex) {
    let mut answer = Hex::None;
    // 0.19.1's wrapping `Int`, or later releases' `Integer` saturated.
    let mut acc: i64 = 0;
    let mut big: u64 = 0;
    while let Some(&w) = src.get(pos) {
        let digit = match w {
            b'0'..=b'9' => w - b'0',
            b'a'..=b'f' => w - b'a' + 10,
            b'A'..=b'F' => w - b'A' + 10,
            _ if dirty(pos) => return (pos, Hex::Bad),
            _ => return (pos, answer),
        };
        if compat == Compat::V0_19_1 {
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

/// What `eatEscape` makes of a backslash.
enum EscapeScan {
    /// The escape's width.
    Ok(usize),
    EndOfFile,
    Problem(E::Escape),
}

/// `eatEscape`, after the backslash at `pos - 1`.
fn eat_escape(p: &P<'_>, pos: usize) -> EscapeScan {
    match p.byte(pos) {
        None => EscapeScan::EndOfFile,
        Some(b'n' | b'r' | b't' | b'"' | b'\'' | b'\\') => EscapeScan::Ok(2),
        Some(b'u') => eat_unicode(p, pos + 1),
        Some(_) => EscapeScan::Problem(E::Escape::EscapeUnknown),
    }
}

fn eat_unicode(p: &P<'_>, pos: usize) -> EscapeScan {
    if p.byte(pos) != Some(b'{') {
        return EscapeScan::Problem(E::Escape::BadUnicodeFormat(2));
    }
    let digit_pos = pos + 1;
    let (new_pos, code) = chomp_hex(p.src(), digit_pos, p.compat, |at| bad_end(p, at));
    let digits = (new_pos - digit_pos) as u32;
    let width = (new_pos - pos) as u32;
    let width = |extra: u32| p.wrap(extra + width);
    if p.byte(new_pos) != Some(b'}') {
        return EscapeScan::Problem(E::Escape::BadUnicodeFormat(width(2)));
    }
    let code = match code {
        Hex::Code(code) if code <= 0x10FFFF => code as u32,
        Hex::Code(_) | Hex::None | Hex::Bad => {
            return EscapeScan::Problem(E::Escape::BadUnicodeCode(width(3)));
        }
    };
    if !(4..=6).contains(&digits) {
        return EscapeScan::Problem(E::Escape::BadUnicodeLength {
            width: width(3),
            digits,
            code,
        });
    }
    EscapeScan::Ok(digits as usize + 4)
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
    to_expectation: impl FnOnce(Position) -> X,
    to_error: impl FnOnce(E::Char, Position) -> X,
) -> Result<(), X> {
    if p.peek(0) != Some(b'\'') {
        return Err(to_expectation(p.cur));
    }
    let mut at = p.here().step(p, 1, 1);
    let mut chars = 0;
    let fail = |p: &mut P<'_>, problem, cur| {
        let x = to_error(problem, cur);
        p.cerr(x)
    };
    loop {
        match p.byte(at.pos) {
            None | Some(b'\n') => return fail(p, E::Char::CharEndless, at.cur),
            Some(b'\'') => {
                at = at.step(p, 1, 1);
                break;
            }
            Some(b'\\') => match eat_escape(p, at.pos + 1) {
                EscapeScan::Ok(width) => {
                    at = at.step(p, width, width as u32);
                    chars += 1;
                }
                EscapeScan::Problem(e) => return fail(p, E::Char::CharEscape(e), at.cur),
                EscapeScan::EndOfFile => return fail(p, E::Char::CharEndless, at.cur),
            },
            Some(0x00..0x80) => {
                at = at.step(p, 1, 1);
                chars += 1;
            }
            Some(_) => match utf8_char(p, at.pos) {
                Utf8::Char(width) => {
                    at = at.step(p, width, 1);
                    chars += 1;
                }
                Utf8::Invalid | Utf8::Truncated => return fail(p, E::Char::CharNotUtf8, at.cur),
            },
        }
    }
    if chars != 1 {
        let width = p.wrap(at.cur.col.wrapping_sub(p.cur.col));
        let start = p.cur;
        return fail(p, E::Char::CharNotString(width), start);
    }
    p.ok_at(at);
    Ok(())
}

/// `String.string`.
pub(super) fn string<X>(
    p: &mut P<'_>,
    to_expectation: impl FnOnce(Position) -> X,
    to_error: impl FnOnce(E::String, Position) -> X,
) -> Result<(), X> {
    if p.peek(0) != Some(b'"') {
        return Err(to_expectation(p.cur));
    }
    let result = if p.peek(1) == Some(b'"') {
        if p.peek(2) == Some(b'"') {
            multi_string(p)
        } else {
            Ok(p.here().step(p, 2, 2))
        }
    } else {
        single_string(p, p.here().step(p, 1, 1))
    };
    match result {
        Ok(end) => {
            p.ok_at(end);
            Ok(())
        }
        Err((problem, at)) => {
            let x = to_error(problem, at);
            p.cerr(x)
        }
    }
}

/// Where a string ends, or what went wrong where.
type Chomped = Result<Loc, (E::String, Position)>;

fn single_string(p: &P<'_>, mut at: Loc) -> Chomped {
    use E::String::{StringEndlessSingle as Endless, StringEscape, StringNotUtf8};
    loop {
        match p.byte(at.pos) {
            None | Some(b'\n') => return Err((Endless, at.cur)),
            Some(b'"') => return Ok(at.step(p, 1, 1)),
            Some(b'\\') => match eat_escape(p, at.pos + 1) {
                EscapeScan::Ok(width) => at = at.step(p, width, width as u32),
                EscapeScan::Problem(e) => return Err((StringEscape(e), at.cur)),
                EscapeScan::EndOfFile => return Err((Endless, p.slide(at.cur, 1))),
            },
            Some(0x00..0x80) => at = at.step(p, 1, 1),
            Some(_) => match utf8_char(p, at.pos) {
                Utf8::Char(width) => at = at.step(p, width, 1),
                Utf8::Invalid => return Err((StringNotUtf8, at.cur)),
                Utf8::Truncated => return Err((Endless, at.cur)),
            },
        }
    }
}

fn multi_string(p: &P<'_>) -> Chomped {
    use E::String::{StringEndlessMulti as Endless, StringEscape, StringNotUtf8};
    let start = p.cur;
    let mut at = p.here().step(p, 3, 3);
    loop {
        match p.byte(at.pos) {
            None => return Err((Endless, start)),
            Some(b'"') if p.byte(at.pos + 1) == Some(b'"') && p.byte(at.pos + 2) == Some(b'"') => {
                return Ok(at.step(p, 3, 3));
            }
            Some(b'\n') => at = at.next_line(p),
            Some(b'\r') => at.pos += 1,
            Some(b'\\') => match eat_escape(p, at.pos + 1) {
                EscapeScan::Ok(width) => at = at.step(p, width, width as u32),
                EscapeScan::Problem(e) => return Err((StringEscape(e), at.cur)),
                EscapeScan::EndOfFile => return Err((Endless, start)),
            },
            Some(0x00..0x80) => at = at.step(p, 1, 1),
            Some(_) => match utf8_char(p, at.pos) {
                Utf8::Char(width) => at = at.step(p, width, 1),
                Utf8::Invalid => return Err((StringNotUtf8, at.cur)),
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
    let mut at = p.here().step(p, 6, 6);
    loop {
        match p.byte(at.pos) {
            None => return p.cerr(E::Expr::ShaderEndless(start)),
            Some(b'|') if p.byte(at.pos + 1) == Some(b']') => break,
            Some(b'\n') => at = at.next_line(p),
            Some(_) => match p.skip_utf8(at.pos) {
                0 => return p.cerr(E::Expr::ShaderNotUtf8(start)),
                width => at = at.step(p, width, 1),
            },
        }
    }
    let text = String::from_utf8_lossy(p.bytes(text_start, at.pos));
    p.ok_at(at.step(p, 2, 2));
    let (offset, message) = match glsl::check(&text) {
        Ok(()) => return Ok(()),
        Err(Rejection::At { offset, message }) => (offset, message),
        // Upstream has no depth limit, so no message to quote.
        Err(Rejection::TooDeep(offset)) => (offset, String::new()),
    };
    let (line, col) = parsec_position(&text[..offset]);
    let at = if p.v1() {
        if line == 1 {
            Position {
                row: start.row,
                col: p.wrap(start.col + 6 + col),
            }
        } else {
            Position {
                row: p.wrap(start.row + line - 1),
                col: p.wrap(col),
            }
        }
    } else if line == 1 {
        p.slide(p.slide(start, 6), col - 1)
    } else {
        let row_start = Position {
            row: start.row + line - 1,
            col: 0,
        };
        p.slide(row_start, col - 1)
    };
    p.cerr(E::Expr::ShaderProblem(message.into(), at))
}

/// Parsec's line and column at the end of `text`: lines from 1, columns
/// from 1 by character, a tab to the next multiple of 8.
fn parsec_position(text: &str) -> (u32, u32) {
    let (mut line, mut col) = (1u32, 1u32);
    for c in text.chars() {
        match c {
            '\n' => (line, col) = (line + 1, 1),
            '\t' => col = col + 8 - (col - 1) % 8,
            _ => col += 1,
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::parsec_position;

    #[test]
    fn parsec_counts_lines_and_columns_from_one_and_tabs_to_eight() {
        for (text, position) in [
            ("", (1, 1)),
            ("ab", (1, 3)),
            ("a\nb", (2, 2)),
            ("\t", (1, 9)),
            ("abc\t", (1, 9)),
            ("a\tb\t", (1, 17)),
            ("é\n\nλx", (3, 3)),
        ] {
            assert_eq!(parsec_position(text), position, "{text:?}");
        }
    }
}
