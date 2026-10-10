//! GHC's `Data.Char` predicates and case mapping, which elm-format's parser and printer
//! and the compiler's `elm.json` parsers lean on.

use icu_casemap::CaseMapper;
use icu_properties::GeneralCategory;
use icu_properties::maps::{self, CodePointMapDataBorrowed};

/// GHC's tables, which are Unicode 15.1 for elm-format 0.8.8 as for the
/// compilers; `icu_properties` is pinned to the release with the same
/// tables.
const GENERAL_CATEGORY: CodePointMapDataBorrowed<'static, GeneralCategory> =
    maps::general_category();

/// GHC's `toLower`: the simple lowercase mapping, one character for one.
pub fn to_lower(c: char) -> char {
    const CASE_MAPPER: CaseMapper = CaseMapper::new();
    CASE_MAPPER.simple_lowercase(c)
}

/// GHC's `isAlpha`: a letter of any case or kind.
pub fn is_alpha(c: char) -> bool {
    matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

/// GHC's `isUpper`: an uppercase or titlecase letter.
pub fn is_upper(c: char) -> bool {
    matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::UppercaseLetter | GeneralCategory::TitlecaseLetter
    )
}

/// GHC's `isLower`: a lowercase letter.
pub fn is_lower(c: char) -> bool {
    GENERAL_CATEGORY.get(c) == GeneralCategory::LowercaseLetter
}

/// GHC's `isAlphaNum`: a letter or a number of any kind.
pub fn is_alpha_num(c: char) -> bool {
    is_alpha(c)
        || matches!(
            GENERAL_CATEGORY.get(c),
            GeneralCategory::DecimalNumber
                | GeneralCategory::LetterNumber
                | GeneralCategory::OtherNumber
        )
}

/// GHC's `isSpace`: the Latin-1 spaces, and the space separators.
pub fn is_space(c: char) -> bool {
    if u32::from(c) <= 0x377 {
        return matches!(c, ' ' | '\t'..='\r' | '\u{A0}');
    }
    GENERAL_CATEGORY.get(c) == GeneralCategory::SpaceSeparator
}

/// GHC's `isPrint`: not a control, format, surrogate, private-use or
/// unassigned character, nor a line or paragraph separator.
pub fn is_print(c: char) -> bool {
    !matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::Surrogate
            | GeneralCategory::PrivateUse
            | GeneralCategory::Unassigned
            | GeneralCategory::LineSeparator
            | GeneralCategory::ParagraphSeparator
    )
}

/// GHC's `isSymbol`: a math, currency, modifier or other symbol.
pub fn is_symbol(c: char) -> bool {
    matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::MathSymbol
            | GeneralCategory::CurrencySymbol
            | GeneralCategory::ModifierSymbol
            | GeneralCategory::OtherSymbol
    )
}

/// How the compilers show bytes that aren't UTF-8 (0.19.2 and later; 0.19.1
/// dies on them): each run of bad bytes reads as one replacement character,
/// which swallows the continuation bytes after it.
pub fn lenient_utf8(mut bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                out.push_str(text);
                return out;
            }
            Err(e) => {
                let (valid, rest) = bytes.split_at(e.valid_up_to());
                out.push_str(std::str::from_utf8(valid).expect("checked as UTF-8"));
                out.push(char::REPLACEMENT_CHARACTER);
                let mut rest = &rest[1..];
                while let [0x80..=0xBF, more @ ..] = rest {
                    rest = more;
                }
                bytes = rest;
            }
        }
    }
}

#[cfg(test)]
mod lenient_tests {
    use super::lenient_utf8;

    #[test]
    fn a_bad_run_reads_as_one_replacement_character() {
        assert_eq!(lenient_utf8(b"s\xffrc"), "s\u{FFFD}rc");
        assert_eq!(lenient_utf8(b"s\xf5\x80\x80\x80rc"), "s\u{FFFD}rc");
        assert_eq!(lenient_utf8(b"s\xe0\x80\x80rc"), "s\u{FFFD}rc");
        assert_eq!(lenient_utf8(b"s\xed\xa0\x80rc"), "s\u{FFFD}rc");
        assert_eq!(lenient_utf8(b"\xff\xff"), "\u{FFFD}\u{FFFD}");
        assert_eq!(lenient_utf8("é".as_bytes()), "é");
    }
}
