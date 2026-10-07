use nomi_base::{ColumnUnit, ElmPosition, LineCol, LineIndex, PositionError, TextSize};
use proptest::prelude::*;

const UNITS: [ColumnUnit; 3] = [ColumnUnit::Utf8, ColumnUnit::Utf16, ColumnUnit::CodePoint];

fn at(n: usize) -> TextSize {
    TextSize::new(n as u32)
}

fn lc(line: u32, col: u32) -> LineCol {
    LineCol { line, col }
}

#[test]
fn elm_positions_follow_lf_crlf_and_bare_cr() {
    let text = "one\r\ntwø\rthree\n";
    let index = LineIndex::new(text);
    let elm = |n| {
        index
            .elm_position(text, at(n), ColumnUnit::CodePoint)
            .unwrap()
    };
    assert_eq!(index.line_count(), 3);
    assert_eq!(elm(0), ElmPosition { row: 1, column: 1 });
    assert_eq!(elm(5), ElmPosition { row: 2, column: 1 });
    // "twø\r" is five bytes, four chars, and the CR does not count.
    assert_eq!(elm(10), ElmPosition { row: 2, column: 4 });
    assert_eq!(elm(text.len()), ElmPosition { row: 3, column: 1 });
}

#[test]
fn utf16_counts_surrogate_pairs() {
    let text = "a😀b\r\nλ";
    let index = LineIndex::new(text);
    let after_emoji = at("a😀".len());
    let u = ColumnUnit::Utf16;
    assert_eq!(index.line_col(text, after_emoji, u), Ok(lc(0, 3)));
    assert_eq!(index.offset(text, lc(0, 3), u), Ok(after_emoji));
    assert_eq!(
        index.offset(text, lc(0, 2), u),
        Err(PositionError::ColumnInsideChar)
    );
    assert_eq!(
        index.line_col(text, after_emoji, ColumnUnit::CodePoint),
        Ok(lc(0, 2))
    );
    assert_eq!(
        index.line_col(text, after_emoji, ColumnUnit::Utf8),
        Ok(lc(0, 5))
    );
}

#[test]
fn line_content_drops_lf_and_crlf_but_keeps_bare_cr() {
    let text = "a\r\nb\rc\n\r";
    let index = LineIndex::new(text);
    let content = |line| &text[index.line_content(text, line).unwrap()];
    assert_eq!(content(0), "a");
    assert_eq!(content(1), "b\rc");
    assert_eq!(content(2), "\r");
    assert_eq!(
        index.line_content(text, 3),
        Err(PositionError::LineOutOfBounds)
    );
}

#[test]
fn empty_text_has_one_line() {
    let index = LineIndex::new("");
    assert_eq!(index.line_count(), 1);
    assert!(index.is_empty());
    assert_eq!(
        index.elm_position("", at(0), ColumnUnit::Utf16),
        Ok(ElmPosition { row: 1, column: 1 })
    );
    assert_eq!(index.offset("", lc(0, 0), ColumnUnit::Utf16), Ok(at(0)));
    assert_eq!(
        index.offset("", lc(0, 1), ColumnUnit::Utf16),
        Err(PositionError::ColumnOutOfBounds)
    );
    assert_eq!(
        index.offset("", lc(1, 0), ColumnUnit::Utf16),
        Err(PositionError::LineOutOfBounds)
    );
}

#[test]
fn offsets_must_be_in_bounds_and_on_char_boundaries() {
    let text = "λ";
    let index = LineIndex::new(text);
    let u = ColumnUnit::CodePoint;
    assert_eq!(
        index.line_col(text, at(1), u),
        Err(PositionError::OffsetInsideChar)
    );
    assert_eq!(
        index.line_col(text, at(3), u),
        Err(PositionError::OffsetOutOfBounds)
    );
}

#[test]
fn a_column_cannot_pass_the_lf() {
    let text = "ab\ncd";
    let index = LineIndex::new(text);
    let u = ColumnUnit::Utf8;
    assert_eq!(index.offset(text, lc(0, 2), u), Ok(at(2)));
    assert_eq!(
        index.offset(text, lc(0, 3), u),
        Err(PositionError::ColumnOutOfBounds)
    );
}

#[test]
#[should_panic(expected = "different text")]
fn querying_another_text_panics() {
    let _ = LineIndex::new("abc").line_col("ab", at(0), ColumnUnit::Utf8);
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof!["[ab\\r\\nλ😀\u{0}]{0,48}", any::<String>(),]
}

fn width(c: char, unit: ColumnUnit) -> u32 {
    match unit {
        ColumnUnit::Utf8 => c.len_utf8() as u32,
        ColumnUnit::Utf16 => c.len_utf16() as u32,
        ColumnUnit::CodePoint => 1,
    }
}

/// Walks the text from the start: the obvious, slow answer.
fn model(text: &str, offset: usize, unit: ColumnUnit, skip_cr: bool) -> LineCol {
    let mut pos = lc(0, 0);
    for c in text[..offset].chars() {
        if c == '\n' {
            pos = lc(pos.line + 1, 0);
        } else if !(skip_cr && c == '\r') {
            pos.col += width(c, unit);
        }
    }
    pos
}

fn boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.char_indices().map(|(i, _)| i).chain([text.len()])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    #[test]
    fn every_boundary_round_trips(text in text()) {
        let index = LineIndex::new(&text);
        for offset in boundaries(&text) {
            for unit in UNITS {
                let pos = index.line_col(&text, at(offset), unit).unwrap();
                prop_assert_eq!(pos, model(&text, offset, unit, false));
                prop_assert_eq!(index.offset(&text, pos, unit), Ok(at(offset)));
                let elm = model(&text, offset, unit, true);
                prop_assert_eq!(
                    index.elm_position(&text, at(offset), unit),
                    Ok(ElmPosition { row: elm.line + 1, column: elm.col + 1 })
                );
            }
        }
    }

    #[test]
    fn any_position_maps_back_or_is_rejected(text in text(), line in 0u32..8, col in 0u32..64) {
        let index = LineIndex::new(&text);
        for unit in UNITS {
            match index.offset(&text, lc(line, col), unit) {
                Ok(offset) => prop_assert_eq!(index.line_col(&text, offset, unit), Ok(lc(line, col))),
                Err(PositionError::LineOutOfBounds) => prop_assert!(line >= index.line_count()),
                Err(PositionError::ColumnOutOfBounds | PositionError::ColumnInsideChar) => {
                    prop_assert!(line < index.line_count())
                }
                Err(e) => prop_assert!(false, "unexpected {e}"),
            }
        }
    }

    #[test]
    fn line_contents_and_terminators_rebuild_the_text(text in text()) {
        let index = LineIndex::new(&text);
        prop_assert_eq!(index.line_count() as usize, text.matches('\n').count() + 1);
        let mut rebuilt = String::new();
        let mut previous_end = 0;
        for line in 0..index.line_count() {
            let range = index.line_content(&text, line).unwrap();
            rebuilt.push_str(&text[previous_end..usize::from(range.start())]);
            let content = &text[range];
            prop_assert!(!content.contains('\n'));
            rebuilt.push_str(content);
            previous_end = range.end().into();
        }
        rebuilt.push_str(&text[previous_end..]);
        prop_assert_eq!(&rebuilt, &text);
        // Each terminator is exactly "\n" or "\r\n".
        for line in 0..index.line_count() - 1 {
            let end = usize::from(index.line_content(&text, line).unwrap().end());
            let next = usize::from(index.line_content(&text, line + 1).unwrap().start());
            let terminator = &text[end..next];
            prop_assert!(terminator == "\n" || terminator == "\r\n", "{:?}", terminator);
        }
    }
}

#[test]
fn file_ids_are_plain_numbers() {
    let id = nomi_base::FileId::from_raw(7);
    assert_eq!(id.raw(), 7);
    assert!(nomi_base::FileId::from_raw(1) < id);
}
