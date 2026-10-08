//! GHC's `Data.Char` predicates, which elm-format's parser and printer
//! lean on.

use icu_properties::GeneralCategory;
use icu_properties::maps::{self, CodePointMapDataBorrowed};

/// GHC's tables, which are Unicode 15.1 for elm-format 0.8.8 as for the
/// compilers; `icu_properties` is pinned to the release with the same
/// tables.
const GENERAL_CATEGORY: CodePointMapDataBorrowed<'static, GeneralCategory> =
    maps::general_category();

/// GHC's `isAlpha`: a letter of any case or kind.
pub(crate) fn is_alpha(c: char) -> bool {
    matches!(
        GENERAL_CATEGORY.get(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

/// GHC's `isSpace`: the Latin-1 spaces, and the space separators.
pub(crate) fn is_space(c: char) -> bool {
    if u32::from(c) <= 0x377 {
        return matches!(c, ' ' | '\t'..='\r' | '\u{A0}');
    }
    GENERAL_CATEGORY.get(c) == GeneralCategory::SpaceSeparator
}

/// GHC's `isPrint`: not a control, format, surrogate, private-use or
/// unassigned character, nor a line or paragraph separator.
pub(crate) fn is_print(c: char) -> bool {
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
