//! UTF-8 as upstream's parsers walk it, a byte at a time.

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
pub fn unchecked_code(bytes: &[u8], width: usize) -> i64 {
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

#[cfg(test)]
mod tests {
    use super::{char_width, unchecked_chars};

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
}
