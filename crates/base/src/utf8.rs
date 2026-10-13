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

#[cfg(test)]
mod tests {
    use super::char_width;

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
}
