//! Protocol positions and byte offsets. Columns count UTF-8 bytes when the
//! client offers that encoding and UTF-16 code units otherwise, as LSP
//! 3.17 negotiates them; nothing past this module sees either.

use lsp_types::{Position, PositionEncodingKind, Range};
use nomi_base::{ColumnUnit, LineIndex, TextRange, TextSize};

/// The encoding the server picks among the client's: UTF-8 if offered,
/// else UTF-16, which every client supports.
pub fn negotiate(offered: Option<&[PositionEncodingKind]>) -> ColumnUnit {
    match offered {
        Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => ColumnUnit::Utf8,
        _ => ColumnUnit::Utf16,
    }
}

pub fn kind(unit: ColumnUnit) -> PositionEncodingKind {
    match unit {
        ColumnUnit::Utf8 => PositionEncodingKind::UTF8,
        ColumnUnit::Utf16 | ColumnUnit::CodePoint => PositionEncodingKind::UTF16,
    }
}

/// One text, for converting positions in it.
pub struct Text<'a> {
    text: &'a str,
    index: LineIndex,
    unit: ColumnUnit,
}

impl<'a> Text<'a> {
    pub fn new(text: &'a str, unit: ColumnUnit) -> Text<'a> {
        Text {
            text,
            index: LineIndex::new(text),
            unit,
        }
    }

    pub fn position(&self, offset: TextSize) -> Position {
        let at = self
            .index
            .line_col(self.text, offset, self.unit)
            .expect("an offset at a character boundary of the text");
        Position::new(at.line, at.col)
    }

    pub fn range(&self, range: TextRange) -> Range {
        Range::new(self.position(range.start()), self.position(range.end()))
    }

    /// The offsets of `range`, as [`Text::offset`] reads each end.
    pub fn text_range(&self, range: Range) -> TextRange {
        let (start, end) = (self.offset(range.start), self.offset(range.end));
        TextRange::new(start, end.max(start))
    }

    /// The offset of `position`. A client may name a column past a line's
    /// end, or a line past the last, which LSP reads as the end of the
    /// line or the text; one inside a character is its start.
    pub fn offset(&self, position: Position) -> TextSize {
        if position.line >= self.index.line_count() {
            return TextSize::of(self.text);
        }
        let line = self
            .index
            .line_content(self.text, position.line)
            .expect("a line of the text");
        let mut col = 0;
        for (at, c) in self.text[line].char_indices() {
            let width = match self.unit {
                ColumnUnit::Utf8 => c.len_utf8(),
                ColumnUnit::Utf16 => c.len_utf16(),
                ColumnUnit::CodePoint => 1,
            } as u32;
            if col + width > position.character {
                return line.start() + TextSize::new(at as u32);
            }
            col += width;
        }
        line.end()
    }
}
