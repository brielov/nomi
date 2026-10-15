//! UTF-8 as upstream's parsers walk it, a byte at a time.

use crate::{Compat, ghc};

/// `getCharWidth`: how many bytes the character starting with `lead`
/// takes, as its high bits say. Upstream fails on a byte no character
/// starts with; here a continuation byte counts as one and any byte from
/// 0xF8 as four, which no `str` holds at a character's start.
pub fn char_width(lead: u8) -> usize {
    match lead {
        0x00..0xC0 => 1,
        0xC0..0xE0 => 2,
        0xE0..0xF0 => 3,
        0xF0.. => 4,
    }
}

/// `chr2`, `chr3` and `chr4` before 0.19.2: the code a lead byte and its
/// continuation bytes assemble, `width` of them, unchecked. Upstream
/// reads past the end for a truncated sequence; here those bytes are
/// zero. The code may be no character at all: negative, a surrogate or
/// past U+10FFFF.
#[inline]
fn unchecked_code(bytes: &[u8], width: usize) -> i64 {
    let byte = |i: usize| i64::from(bytes.get(i).copied().unwrap_or(0));
    let tail = |i: usize, shift: u32| (byte(i) - 0x80) << shift;
    match width {
        2 => ((byte(0) - 0xC0) << 6) + tail(1, 0),
        3 => ((byte(0) - 0xE0) << 12) + tail(1, 6) + tail(2, 0),
        _ => ((byte(0) - 0xF0) << 18) + tail(1, 12) + tail(2, 6) + tail(3, 0),
    }
}

/// `toChars` before 0.19.2: the codes of `bytes`, each as wide as its
/// lead byte says and assembled by [`unchecked_code`]. A byte below 0xC0
/// is a character of its own.
#[inline]
pub fn unchecked_chars(bytes: &[u8]) -> impl Iterator<Item = i64> + '_ {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        let &lead = rest.first()?;
        let width = char_width(lead);
        let code = if width == 1 {
            i64::from(lead)
        } else {
            unchecked_code(rest, width)
        };
        rest = rest.get(width..).unwrap_or_default();
        Some(code)
    })
}

/// `chompUpper`: the width of the character `rest` starts with if it can
/// start a capitalized name, or zero.
pub fn upper_width(rest: &[u8], compat: Compat) -> usize {
    chomp(rest, |b| b.is_ascii_uppercase(), ghc::is_upper, compat)
}

/// `chompLower`: as [`upper_width`], for a lower-case name.
pub fn lower_width(rest: &[u8], compat: Compat) -> usize {
    chomp(rest, |b| b.is_ascii_lowercase(), ghc::is_lower, compat)
}

/// `chompInner`: as [`upper_width`], for a character that continues a
/// name, `[A-Za-z0-9_]` or a letter outside ASCII.
pub fn inner_width(rest: &[u8], compat: Compat) -> usize {
    chomp(
        rest,
        |b| b.is_ascii_alphanumeric() || b == b'_',
        ghc::is_alpha,
        compat,
    )
}

/// `AST/Prim/Variable.hs`'s `chomp`: the width of the character `rest`
/// starts with if it is in a class, which takes ASCII by `ascii` and
/// anything else by `unicode`; zero if it isn't. 0.19.2 and later decode
/// strict UTF-8; 0.19.1's `get*WidthHelp` assembles the character from
/// its lead byte's width unchecked. A sequence `rest` cuts off is in no
/// class.
fn chomp(
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
        0x00..0xC0 | 0xF8.. => return 0,
        lead => char_width(lead),
    };
    let Some(bytes) = rest.get(..width) else {
        return 0;
    };
    let c = match compat {
        Compat::V0_19_1 => u32::try_from(unchecked_code(bytes, width))
            .ok()
            .and_then(char::from_u32),
        Compat::V0_19_2 | Compat::V0_19_3 => std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.chars().next()),
    };
    if c.is_some_and(unicode) { width } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::{char_width, inner_width, unchecked_chars, upper_width};
    use crate::Compat;

    #[test]
    fn a_lead_byte_gives_its_characters_width() {
        for c in ['a', '\u{7f}', 'é', 'λ', '€', '\u{ffff}', '😀'] {
            let mut bytes = [0; 4];
            let lead = c.encode_utf8(&mut bytes).as_bytes()[0];
            assert_eq!(char_width(lead), c.len_utf8(), "{c:?}");
        }
        assert_eq!(char_width(0x80), 1);
        assert_eq!(char_width(0xFF), 4);
    }

    #[test]
    fn unchecked_chars_assemble_what_the_lead_bytes_say() {
        let codes: Vec<i64> = unchecked_chars("aé€😀".as_bytes()).collect();
        assert_eq!(codes, [0x61, 0xE9, 0x20AC, 0x1F600]);
        // A stray continuation byte stands alone; a lead byte takes the
        // next byte, checked or not; the missing bytes of a cut-off
        // sequence are zero.
        let codes: Vec<i64> = unchecked_chars(b"\x80\xC3\x09\xE2").collect();
        assert_eq!(
            codes,
            [
                0x80,
                (0x03 << 6) + 0x09 - 0x80,
                (0x02 << 12) - (0x80 << 6) - 0x80
            ]
        );
    }

    #[test]
    fn name_classes_take_whole_characters() {
        for compat in Compat::ALL {
            assert_eq!(upper_width("Éa".as_bytes(), compat), 2);
            assert_eq!(upper_width(b"a", compat), 0);
            assert_eq!(inner_width(b"_", compat), 1);
            assert_eq!(inner_width("→".as_bytes(), compat), 0);
            // A sequence cut off by the end is no character.
            assert_eq!(inner_width(b"\xC5", compat), 0);
        }
        // 0.19.1 assembles `C3 09` into `I` without checking it.
        assert_eq!(upper_width(b"\xC3\x09", Compat::V0_19_1), 2);
        assert_eq!(upper_width(b"\xC3\x09", Compat::V0_19_3), 0);
    }
}
