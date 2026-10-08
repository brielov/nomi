//! `Parse/Literal.hs`, `Parse/Number.hs` and `Parse/String.hs`: the
//! parsers that read bytes rather than characters, so count a column a
//! character whatever it is, a tab included.

use nomi_base::ghc;

use super::engine::{P, R};

/// `literal`: a number, string or character, failing where it started
/// if it is none.
pub(super) fn literal(p: &mut P<'_>) -> R<()> {
    p.choice(&mut [&mut num, &mut string, &mut character])
}

fn num(p: &mut P<'_>) -> R<()> {
    p.option(|p| {
        p.attempt(|p| {
            p.char('-')?;
            p.look_ahead(P::digit)
        })
    })?;
    number(p)
}

// NUMBERS

/// Where a number ended, or the error ending it.
enum Outcome {
    Ok(usize),
    Err(usize),
}

/// `Parse.Number.number`.
fn number(p: &mut P<'_>) -> R<()> {
    let start = p.pos;
    let outcome = match p.peek(0) {
        Some(b'0') => chomp_zero(p.src(), start + 1),
        Some(b @ b'1'..=b'9') => chomp_int(p.src(), start + 1, b),
        _ => return p.fail(),
    };
    // `fromIntegral` to a `Word16`, which wraps.
    #[allow(clippy::cast_possible_truncation, reason = "Word16 arithmetic")]
    let col = |at: usize| p.col.wrapping_add((at - start) as u16);
    match outcome {
        Outcome::Ok(end) => {
            let col = col(end);
            p.advance(end - start, col);
            Ok(())
        }
        Outcome::Err(at) => {
            let (row, col) = (p.row, col(at));
            p.cerr(row, col)
        }
    }
}

/// `isDirtyEnd`: a character that would continue a name.
fn is_dirty_end(src: &str, pos: usize) -> bool {
    match src[pos..].chars().next() {
        Some(c) if c.is_ascii() => c.is_ascii_alphanumeric() || c == '_',
        Some(c) => ghc::is_alpha(c),
        None => false,
    }
}

fn byte(src: &str, pos: usize) -> Option<u8> {
    src.as_bytes().get(pos).copied()
}

fn is_digit(src: &str, pos: usize) -> bool {
    byte(src, pos).is_some_and(|b| b.is_ascii_digit())
}

fn chomp_int(src: &str, mut pos: usize, _first: u8) -> Outcome {
    loop {
        match byte(src, pos) {
            None => return Outcome::Ok(pos),
            Some(b'0'..=b'9') => pos += 1,
            Some(b'.') if byte(src, pos + 1) != Some(b'.') => return chomp_fraction(src, pos),
            Some(b'e' | b'E') => return chomp_exponent(src, pos + 1),
            Some(_) if is_dirty_end(src, pos) => return Outcome::Err(pos),
            Some(_) => return Outcome::Ok(pos),
        }
    }
}

fn chomp_fraction(src: &str, pos: usize) -> Outcome {
    if !is_digit(src, pos + 1) {
        return Outcome::Err(pos);
    }
    let mut pos = pos + 2;
    loop {
        match byte(src, pos) {
            None => return Outcome::Ok(pos),
            Some(b'0'..=b'9') => pos += 1,
            Some(b'e' | b'E') => return chomp_exponent(src, pos + 1),
            Some(_) if is_dirty_end(src, pos) => return Outcome::Err(pos),
            Some(_) => return Outcome::Ok(pos),
        }
    }
}

fn chomp_exponent(src: &str, pos: usize) -> Outcome {
    let mut pos = match byte(src, pos) {
        Some(b'0'..=b'9') => pos + 1,
        Some(b'+' | b'-') if is_digit(src, pos + 1) => pos + 2,
        _ => return Outcome::Err(pos),
    };
    while is_digit(src, pos) {
        pos += 1;
    }
    Outcome::Ok(pos)
}

fn chomp_zero(src: &str, pos: usize) -> Outcome {
    match byte(src, pos) {
        None => Outcome::Ok(pos),
        Some(b'x') => {
            let (end, answer) = chomp_hex(src, pos + 1);
            if answer < 0 {
                Outcome::Err(end)
            } else {
                Outcome::Ok(end)
            }
        }
        Some(b'.') => chomp_fraction(src, pos),
        Some(b'0'..=b'9') => Outcome::Err(pos),
        Some(_) if is_dirty_end(src, pos) => Outcome::Err(pos),
        Some(_) => Outcome::Ok(pos),
    }
}

/// `chompHex`, in `Int64`, which wraps: -1 if there were no digits, -2
/// if they ran into more of a name.
fn chomp_hex(src: &str, mut pos: usize) -> (usize, i64) {
    let mut answer = -1;
    loop {
        let Some(word) = byte(src, pos) else {
            return (pos, answer);
        };
        let digit = match word {
            b'0'..=b'9' => Some(word - b'0'),
            b'a'..=b'f' => Some(word - b'a' + 10),
            b'A'..=b'F' => Some(word - b'A' + 10),
            _ => None,
        };
        let acc = answer.max(0);
        let new = match digit {
            Some(d) => acc.wrapping_mul(16).wrapping_add(i64::from(d)),
            None if is_dirty_end(src, pos) => -2,
            None => -1,
        };
        if new < 0 {
            return (pos, if new == -1 { answer } else { -2 });
        }
        answer = new;
        pos += 1;
    }
}

// STRINGS

/// What `eatEscape` makes of what follows a backslash at `pos`.
enum Escape {
    /// So many bytes from the backslash, a column each.
    Ok(usize),
    EndOfFile,
    Problem,
}

/// `eatEscape`, `pos` past the backslash.
fn eat_escape(src: &str, pos: usize) -> Escape {
    match byte(src, pos) {
        None => Escape::EndOfFile,
        Some(b'n' | b'r' | b't' | b'"' | b'\'' | b'\\') => Escape::Ok(2),
        Some(b'x') => {
            // `eatPre019Unicode`.
            let digits = pos + 1;
            if digits >= src.len() {
                return Escape::Problem;
            }
            let (end, _) = chomp_hex(src, digits);
            if end >= src.len() {
                Escape::Problem
            } else {
                Escape::Ok(2 + end - digits)
            }
        }
        Some(b'u') => {
            // `eatUnicode`.
            if byte(src, pos + 1) != Some(b'{') {
                return Escape::Problem;
            }
            let digits = pos + 2;
            let (end, code) = chomp_hex(src, digits);
            let n = end - digits;
            if byte(src, end) != Some(b'}')
                || !(0..=0x10_FFFF).contains(&code)
                || !(4..=6).contains(&n)
            {
                Escape::Problem
            } else {
                Escape::Ok(n + 4)
            }
        }
        Some(_) => Escape::Problem,
    }
}

/// The width of the character starting with `word`.
fn char_width(word: u8) -> usize {
    match word {
        0..0x80 => 1,
        0x80..0xE0 => 2,
        0xE0..0xF0 => 3,
        _ => 4,
    }
}

#[allow(clippy::cast_possible_truncation, reason = "Word16 arithmetic")]
fn cols(n: usize) -> u16 {
    n as u16
}

/// `Parse.String.character`.
fn character(p: &mut P<'_>) -> R<()> {
    if p.peek(0) != Some(b'\'') {
        return p.fail();
    }
    let src = p.src();
    let (row, start) = (p.row, p.col);
    let mut pos = p.pos + 1;
    let mut col = start.wrapping_add(1);
    let mut chars: u16 = 0;
    loop {
        match byte(src, pos) {
            None | Some(b'\n') => return p.cerr(row, col),
            Some(b'\'') => break,
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => {
                    pos += delta;
                    col = col.wrapping_add(cols(delta));
                }
                Escape::EndOfFile | Escape::Problem => return p.cerr(row, col),
            },
            Some(word) => {
                pos += char_width(word);
                col = col.wrapping_add(1);
            }
        }
        chars = chars.wrapping_add(1);
    }
    if chars != 1 {
        return p.cerr(row, start);
    }
    p.advance(pos + 1 - p.pos, col.wrapping_add(1));
    Ok(())
}

/// `Parse.String.string`.
fn string(p: &mut P<'_>) -> R<()> {
    if p.peek(0) != Some(b'"') {
        return p.fail();
    }
    let src = p.src();
    let (row, col) = (p.row, p.col);
    let outcome = if p.peek(1) != Some(b'"') {
        single_string(src, p.pos + 1, row, col.wrapping_add(1))
    } else if p.peek(2) != Some(b'"') {
        Ok((p.pos + 2, row, col.wrapping_add(2)))
    } else {
        multi_string(src, p.pos + 3, row, col.wrapping_add(3), (row, col))
    };
    match outcome {
        Ok((end, row, col)) => {
            p.advance_to(end - p.pos, row, col);
            Ok(())
        }
        Err((row, col)) => p.cerr(row, col),
    }
}

type Chomped = Result<(usize, u16, u16), (u16, u16)>;

fn single_string(src: &str, mut pos: usize, mut row: u16, mut col: u16) -> Chomped {
    loop {
        match byte(src, pos) {
            None => return Err((row, col)),
            Some(b'"') => return Ok((pos + 1, row, col.wrapping_add(1))),
            Some(b'\n') => (pos, row, col) = (pos + 1, row.wrapping_add(1), 1),
            Some(b'\r') if byte(src, pos + 1) == Some(b'\n') => {
                (pos, row, col) = (pos + 2, row.wrapping_add(1), 1);
            }
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => (pos, col) = (pos + delta, col.wrapping_add(cols(delta))),
                Escape::Problem => return Err((row, col)),
                Escape::EndOfFile => return Err((row, col.wrapping_add(1))),
            },
            Some(word) => (pos, col) = (pos + char_width(word), col.wrapping_add(1)),
        }
    }
}

fn multi_string(
    src: &str,
    mut pos: usize,
    mut row: u16,
    mut col: u16,
    start: (u16, u16),
) -> Chomped {
    loop {
        match byte(src, pos) {
            None => return Err(start),
            Some(b'"') if src[pos..].starts_with("\"\"\"") => {
                return Ok((pos + 3, row, col.wrapping_add(3)));
            }
            Some(b'\n') => (pos, row, col) = (pos + 1, row.wrapping_add(1), 1),
            Some(b'\r') if byte(src, pos + 1) == Some(b'\n') => {
                (pos, row, col) = (pos + 2, row.wrapping_add(1), 1);
            }
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => (pos, col) = (pos + delta, col.wrapping_add(cols(delta))),
                Escape::Problem => return Err((row, col)),
                Escape::EndOfFile => return Err(start),
            },
            Some(word) => (pos, col) = (pos + char_width(word), col.wrapping_add(1)),
        }
    }
}
