//! Conversions between byte offsets and line/column positions.
//!
//! Lines end at LF only, as in Elm: a CRLF ends one line, and a bare CR
//! stays inside its line. Columns are counted in a [`ColumnUnit`] chosen by
//! the caller: LSP uses UTF-16, and every elm release counts code points.

use std::fmt;

use text_size::{TextRange, TextSize};

/// What a column counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnUnit {
    /// UTF-8 bytes.
    Utf8,
    /// UTF-16 code units, as LSP counts them.
    Utf16,
    /// Unicode scalar values.
    CodePoint,
}

/// A zero-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// A one-based row and column, as Elm prints them in diagnostics. A CR
/// does not advance Elm's column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElmPosition {
    pub row: u32,
    pub column: u32,
}

/// Why an offset or a position has no counterpart in a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionError {
    OffsetOutOfBounds,
    OffsetInsideChar,
    LineOutOfBounds,
    ColumnOutOfBounds,
    ColumnInsideChar,
}

/// The start of every line in one text. It stores no text, so every query
/// takes the text it was built from; passing another one panics if the
/// length differs and gives wrong answers otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineIndex {
    /// Always starts with zero; one entry per line.
    starts: Vec<TextSize>,
    len: TextSize,
}

impl LineIndex {
    /// Panics if `text` is 4 GiB or longer; the VFS rejects such files.
    pub fn new(text: &str) -> LineIndex {
        let len = TextSize::try_from(text.len()).expect("text shorter than 4 GiB");
        let mut starts = vec![TextSize::new(0)];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| TextSize::new(i as u32 + 1)),
        );
        LineIndex { starts, len }
    }

    /// At least one: an empty text has one empty line.
    pub fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }

    /// The line holding `offset`; past the text, its last line.
    pub fn line(&self, offset: TextSize) -> u32 {
        (self.starts.partition_point(|&s| s <= offset) - 1) as u32
    }

    /// The position of `offset`. An offset between a CR and its LF counts
    /// the CR, so every char boundary has its own position.
    pub fn line_col(
        &self,
        text: &str,
        offset: TextSize,
        unit: ColumnUnit,
    ) -> Result<LineCol, PositionError> {
        let (line, prefix) = self.prefix(text, offset)?;
        Ok(LineCol {
            line,
            col: prefix.chars().map(|c| char_width(c, unit)).sum(),
        })
    }

    /// The offset of `pos`, the inverse of [`LineIndex::line_col`]. The
    /// column may point at the line's LF but not past it.
    pub fn offset(
        &self,
        text: &str,
        pos: LineCol,
        unit: ColumnUnit,
    ) -> Result<TextSize, PositionError> {
        let line = self.line_range(text, pos.line)?;
        let mut col = 0;
        for (i, c) in text[line].char_indices() {
            if col == pos.col {
                return Ok(line.start() + TextSize::new(i as u32));
            }
            col += char_width(c, unit);
            if col > pos.col {
                return Err(PositionError::ColumnInsideChar);
            }
        }
        if col == pos.col {
            Ok(line.end())
        } else {
            Err(PositionError::ColumnOutOfBounds)
        }
    }

    /// The position Elm would report for `offset`, its column in code
    /// points other than CR.
    pub fn elm_position(&self, text: &str, offset: TextSize) -> Result<ElmPosition, PositionError> {
        let (line, prefix) = self.prefix(text, offset)?;
        Ok(ElmPosition {
            row: line + 1,
            column: prefix.chars().filter(|&c| c != '\r').count() as u32 + 1,
        })
    }

    /// The offset of `pos` as Elm counts columns, in code points other
    /// than CR, the inverse of [`LineIndex::elm_position`] made
    /// zero-based: past its line's end, the end before its LF, and past
    /// the last line, the text's end.
    pub fn elm_offset(&self, text: &str, pos: LineCol) -> TextSize {
        let Ok(line) = self.line_range(text, pos.line) else {
            return self.len;
        };
        let mut col = 0;
        for (i, c) in text[line].char_indices() {
            if col == pos.col {
                return line.start() + TextSize::new(i as u32);
            }
            if c != '\r' {
                col += 1;
            }
        }
        line.end()
    }

    /// The offset of `pos` as LSP reads a position: past its line's end,
    /// the end before its LF or CRLF; past the last line, the text's end;
    /// inside a character, that character's start.
    pub fn clamped_offset(&self, text: &str, pos: LineCol, unit: ColumnUnit) -> TextSize {
        let Ok(line) = self.line_content(text, pos.line) else {
            return self.len;
        };
        let mut col = 0;
        for (i, c) in text[line].char_indices() {
            col += char_width(c, unit);
            if col > pos.col {
                return line.start() + TextSize::new(i as u32);
            }
        }
        line.end()
    }

    /// The text of `line` without its LF or CRLF; a bare CR stays.
    pub fn line_content(&self, text: &str, line: u32) -> Result<TextRange, PositionError> {
        let range = self.line_range(text, line)?;
        let (start, mut end) = (range.start(), range.end());
        let ends_in_lf = (line as usize + 1) < self.starts.len();
        if ends_in_lf && end > start && text.as_bytes()[usize::from(end) - 1] == b'\r' {
            end -= TextSize::new(1);
        }
        Ok(TextRange::new(start, end))
    }

    /// The line holding `offset` and the text from the line's start to it.
    fn prefix<'t>(&self, text: &'t str, offset: TextSize) -> Result<(u32, &'t str), PositionError> {
        self.check(text);
        if offset > self.len {
            return Err(PositionError::OffsetOutOfBounds);
        }
        if !text.is_char_boundary(offset.into()) {
            return Err(PositionError::OffsetInsideChar);
        }
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        Ok((line as u32, &text[usize::from(start)..usize::from(offset)]))
    }

    /// `line`, up to its LF or the end of the text.
    fn line_range(&self, text: &str, line: u32) -> Result<TextRange, PositionError> {
        self.check(text);
        let line = line as usize;
        let start = *self
            .starts
            .get(line)
            .ok_or(PositionError::LineOutOfBounds)?;
        let end = (self.starts.get(line + 1)).map_or(self.len, |&next| next - TextSize::new(1));
        Ok(TextRange::new(start, end))
    }

    fn check(&self, text: &str) {
        assert_eq!(
            text.len(),
            usize::from(self.len),
            "LineIndex queried with a different text"
        );
    }
}

fn char_width(c: char, unit: ColumnUnit) -> u32 {
    match unit {
        ColumnUnit::Utf8 => c.len_utf8() as u32,
        ColumnUnit::Utf16 => c.len_utf16() as u32,
        ColumnUnit::CodePoint => 1,
    }
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PositionError::OffsetOutOfBounds => "offset is past the end of the text",
            PositionError::OffsetInsideChar => "offset is inside a UTF-8 character",
            PositionError::LineOutOfBounds => "line is past the last line",
            PositionError::ColumnOutOfBounds => "column is past the end of the line",
            PositionError::ColumnInsideChar => "column is inside a character",
        })
    }
}

impl std::error::Error for PositionError {}
