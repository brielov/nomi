//! `Parse/Literal.hs`, `Parse/Number.hs` and `Parse/String.hs`: the
//! parsers that read bytes rather than characters, so count a column a
//! character whatever it is, a tab included.

use nomi_base::{Compat, utf8};

use super::engine::{P, Pos, R};
use crate::elm::{Dots, chomp_hex, chomp_number, eat_unicode};

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

/// `Parse.Number.number`.
fn number(p: &mut P<'_>) -> R<()> {
    if !p.peek(0).is_some_and(|b| b.is_ascii_digit()) {
        return p.fail();
    }
    let start = p.pos;
    let scanned = chomp_number(p.src().as_bytes(), start, Compat::V0_19_1, Dots::Range);
    // `fromIntegral` to a `Word16`, which wraps.
    let at = |end: usize| p.at.right(cols(end - start));
    match scanned {
        Ok((end, _)) => {
            let to = at(end);
            p.advance(end - start, to);
            Ok(())
        }
        Err((end, _)) => {
            let to = at(end);
            p.cerr(to)
        }
    }
}

fn byte(src: &str, pos: usize) -> Option<u8> {
    src.as_bytes().get(pos).copied()
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
            let (end, _) = chomp_hex(src.as_bytes(), digits, Compat::V0_19_1);
            if end >= src.len() {
                Escape::Problem
            } else {
                Escape::Ok(2 + end - digits)
            }
        }
        Some(b'u') => eat_unicode(src.as_bytes(), pos + 1, Compat::V0_19_1)
            .map_or(Escape::Problem, Escape::Ok),
        Some(_) => Escape::Problem,
    }
}

#[expect(clippy::cast_possible_truncation, reason = "Word16 arithmetic")]
fn cols(n: usize) -> u16 {
    n as u16
}

/// `Parse.String.character`.
fn character(p: &mut P<'_>) -> R<()> {
    if p.peek(0) != Some(b'\'') {
        return p.fail();
    }
    let src = p.src();
    let start = p.at;
    let mut pos = p.pos + 1;
    let mut at = start.right(1);
    let mut chars: u16 = 0;
    loop {
        match byte(src, pos) {
            None | Some(b'\n') => return p.cerr(at),
            Some(b'\'') => break,
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => {
                    pos += delta;
                    at = at.right(cols(delta));
                }
                Escape::EndOfFile | Escape::Problem => return p.cerr(at),
            },
            // `pos` is at a character's start: escapes and characters end
            // on one.
            Some(lead) => {
                pos += utf8::char_width(lead);
                at = at.right(1);
            }
        }
        chars = chars.wrapping_add(1);
    }
    if chars != 1 {
        return p.cerr(start);
    }
    p.advance(pos + 1 - p.pos, at.right(1));
    Ok(())
}

/// `Parse.String.string`.
fn string(p: &mut P<'_>) -> R<()> {
    if p.peek(0) != Some(b'"') {
        return p.fail();
    }
    let src = p.src();
    let start = p.at;
    let chomped = if p.peek(1) != Some(b'"') {
        single_string(src, p.pos + 1, start.right(1))
    } else if p.peek(2) != Some(b'"') {
        Ok((p.pos + 2, start.right(2)))
    } else {
        multi_string(src, p.pos + 3, start.right(3), start)
    };
    match chomped {
        Ok((end, to)) => {
            p.advance(end - p.pos, to);
            Ok(())
        }
        Err(at) => p.cerr(at),
    }
}

/// Where a string ends, and the position there, or where the error is.
type Chomped = Result<(usize, Pos), Pos>;

fn single_string(src: &str, mut pos: usize, mut at: Pos) -> Chomped {
    loop {
        match byte(src, pos) {
            None => return Err(at),
            Some(b'"') => return Ok((pos + 1, at.right(1))),
            Some(b'\n') => (pos, at) = (pos + 1, at.next_row()),
            Some(b'\r') if byte(src, pos + 1) == Some(b'\n') => {
                (pos, at) = (pos + 2, at.next_row());
            }
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => (pos, at) = (pos + delta, at.right(cols(delta))),
                Escape::Problem => return Err(at),
                Escape::EndOfFile => return Err(at.right(1)),
            },
            Some(lead) => (pos, at) = (pos + utf8::char_width(lead), at.right(1)),
        }
    }
}

fn multi_string(src: &str, mut pos: usize, mut at: Pos, start: Pos) -> Chomped {
    loop {
        match byte(src, pos) {
            None => return Err(start),
            Some(b'"') if src[pos..].starts_with("\"\"\"") => {
                return Ok((pos + 3, at.right(3)));
            }
            Some(b'\n') => (pos, at) = (pos + 1, at.next_row()),
            Some(b'\r') if byte(src, pos + 1) == Some(b'\n') => {
                (pos, at) = (pos + 2, at.next_row());
            }
            Some(b'\\') => match eat_escape(src, pos + 1) {
                Escape::Ok(delta) => (pos, at) = (pos + delta, at.right(cols(delta))),
                Escape::Problem => return Err(at),
                Escape::EndOfFile => return Err(start),
            },
            Some(lead) => (pos, at) = (pos + utf8::char_width(lead), at.right(1)),
        }
    }
}
