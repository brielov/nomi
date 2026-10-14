//! String and character literals as the optimized graph keeps them
//! (`Parse/String.hs` and `Elm.String.fromChunks`), from their tokens as
//! written, quotes included. A string is JavaScript-ready text: 0.19.1
//! keeps the escapes `\n`, `\r`, `\t`, `\"`, `\'` and `\\` as written,
//! and later releases write every escape as `\uXXXX`. A character is the
//! same text in 0.19.1, and the character itself later.

use std::fmt::Write as _;

use nomi_base::Compat;
use nomi_syntax::literal_body;

/// The text a string literal's token stands for.
pub(super) fn string(compat: Compat, token: &str) -> String {
    let body = literal_body(token).unwrap_or(token);
    let multi = token.starts_with("\"\"\"");
    let v1 = compat == Compat::V0_19_1;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' if v1 => out.push_str("\\'"),
            '\'' => code_point(&mut out, 0x27),
            '"' if multi && !v1 => code_point(&mut out, 0x22),
            '\n' if multi && v1 => out.push_str("\\n"),
            '\n' if multi => code_point(&mut out, 0x0A),
            '\r' if multi && v1 => out.push_str("\\r"),
            '\r' if multi => code_point(&mut out, 0x0D),
            '\\' => match escape(&mut chars) {
                Escape::Unicode(code) => code_point(&mut out, code),
                Escape::Plain(c) if v1 => {
                    out.push('\\');
                    out.push(c);
                }
                Escape::Plain(c) => code_point(&mut out, plain_code(c)),
            },
            c => out.push(c),
        }
    }
    out
}

/// What a character literal's token stands for.
pub(super) fn char(compat: Compat, token: &str) -> String {
    let body = literal_body(token).unwrap_or(token);
    let mut chars = body.chars();
    let Some(c) = chars.next() else {
        return String::new();
    };
    let v1 = compat == Compat::V0_19_1;
    match c {
        '"' if v1 => "\\\"".to_owned(),
        '\\' => match escape(&mut chars) {
            Escape::Unicode(code) if v1 => {
                let mut out = String::new();
                code_point(&mut out, code);
                out
            }
            Escape::Plain(c) if v1 => format!("\\{c}"),
            Escape::Unicode(code) => char::from_u32(code).map(String::from).unwrap_or_default(),
            Escape::Plain(c) => char::from_u32(plain_code(c))
                .map(String::from)
                .unwrap_or_default(),
        },
        c => c.to_string(),
    }
}

/// What follows a backslash.
enum Escape {
    /// One of `n`, `r`, `t`, `"`, `'` and `\`.
    Plain(char),
    /// `u{…}`, by its code point.
    Unicode(u32),
}

/// The escape after a backslash, which the parser has checked.
fn escape(chars: &mut std::str::Chars<'_>) -> Escape {
    match chars.next() {
        Some('u') => {
            chars.next(); // `{`
            let mut code = 0;
            for c in chars.by_ref() {
                match c.to_digit(16) {
                    Some(digit) => code = code * 16 + digit,
                    None => break, // `}`
                }
            }
            Escape::Unicode(code)
        }
        Some(c) => Escape::Plain(c),
        None => Escape::Plain('\\'),
    }
}

/// The code point of a plain escape's character.
fn plain_code(c: char) -> u32 {
    match c {
        'n' => 0x0A,
        'r' => 0x0D,
        't' => 0x09,
        c => u32::from(c),
    }
}

/// `writeCode`: `\uXXXX` in upper case, or a surrogate pair from `0xFFFF`
/// on, which upstream counts as past the first plane. `0xFFFF` itself
/// takes the pair of offset -1: 0.19.1's `divMod` floors, and later
/// releases' unsigned arithmetic wraps to the same last 16 bits.
fn code_point(out: &mut String, code: u32) {
    // Writing to a `String` can't fail.
    let _ = if code < 0xFFFF {
        write!(out, "\\u{code:04X}")
    } else {
        let offset = i64::from(code) - 0x10000;
        let hi = offset.div_euclid(0x400) + 0xD800;
        let lo = offset.rem_euclid(0x400) + 0xDC00;
        write!(out, "\\u{:04X}\\u{:04X}", hi & 0xFFFF, lo & 0xFFFF)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_keep_escapes_as_each_release_does() {
        let v1 = Compat::V0_19_1;
        let v2 = Compat::V0_19_2;
        let token = r#""esc\"aped\n\t\\""#;
        assert_eq!(string(v1, token), r#"esc\"aped\n\t\\"#);
        assert_eq!(string(v2, token), r"esc\u0022aped\u000A\u0009\u005C");
        assert_eq!(string(v1, r#""\u{00E9}t""#), r"\u00E9t");
        assert_eq!(string(v1, r#""it's""#), r"it\'s");
        assert_eq!(string(v1, r#""\u{FFFF}""#), r"\uD7FF\uDFFF");
        assert_eq!(string(v2, r#""\u{FFFF}""#), r"\uD7FF\uDFFF");
        assert_eq!(char(v1, r"'\u{FFFF}'"), r"\uD7FF\uDFFF");
        assert_eq!(string(v2, r#""it's""#), r"it\u0027s");
        assert_eq!(string(v1, "\"\"\"a\n\"b\"\"\""), "a\\n\"b");
        assert_eq!(string(v2, "\"\"\"a\n\"b\"\"\""), r"a\u000A\u0022b");
        assert_eq!(string(v1, r#""\u{1F600}""#), r"\uD83D\uDE00");
        assert_eq!(string(v1, "\"\u{e9}\""), "\u{e9}");
    }

    #[test]
    fn characters_are_text_in_0_19_1_and_themselves_later() {
        assert_eq!(char(Compat::V0_19_1, r"'\''"), r"\'");
        assert_eq!(char(Compat::V0_19_1, "'\"'"), "\\\"");
        assert_eq!(char(Compat::V0_19_1, r"'\u{1F600}'"), r"\uD83D\uDE00");
        assert_eq!(char(Compat::V0_19_2, r"'\t'"), "\t");
        assert_eq!(char(Compat::V0_19_3, r"'\u{1F600}'"), "\u{1F600}");
        assert_eq!(char(Compat::V0_19_2, "'\u{e9}'"), "\u{e9}");
    }
}
