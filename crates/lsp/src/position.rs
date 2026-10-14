//! Protocol positions and byte offsets. Columns count UTF-8 bytes when the
//! client offers that encoding and UTF-16 code units otherwise, as LSP
//! 3.17 negotiates them; nothing past this module sees either.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_types::{Location, Position, PositionEncodingKind, Range, TextEdit, Url};
use nomi_base::{ColumnUnit, LineCol, LineIndex, TextRange, TextSize};
use nomi_engine::Snapshot;

/// The encoding the server picks among the client's, and the unit its
/// columns count: UTF-8 if offered, else UTF-16, which every client
/// supports.
pub(crate) fn negotiate(
    offered: Option<&[PositionEncodingKind]>,
) -> (PositionEncodingKind, ColumnUnit) {
    if offered.is_some_and(|kinds| kinds.contains(&PositionEncodingKind::UTF8)) {
        (PositionEncodingKind::UTF8, ColumnUnit::Utf8)
    } else {
        (PositionEncodingKind::UTF16, ColumnUnit::Utf16)
    }
}

/// One text, for converting positions in it.
pub(crate) struct Text<'a> {
    text: Cow<'a, str>,
    index: LineIndex,
    unit: ColumnUnit,
}

impl<'a> Text<'a> {
    pub(crate) fn new(text: impl Into<Cow<'a, str>>, unit: ColumnUnit) -> Text<'a> {
        let text = text.into();
        Text {
            index: LineIndex::new(&text),
            text,
            unit,
        }
    }

    pub(crate) fn position(&self, offset: TextSize) -> Position {
        let at = self
            .index
            .line_col(&self.text, offset, self.unit)
            .expect("an offset at a character boundary of the text");
        Position::new(at.line, at.col)
    }

    pub(crate) fn range(&self, range: TextRange) -> Range {
        Range::new(self.position(range.start()), self.position(range.end()))
    }

    /// The offsets of `range`, as [`Text::offset`] reads each end.
    pub(crate) fn text_range(&self, range: Range) -> TextRange {
        let (start, end) = (self.offset(range.start), self.offset(range.end));
        TextRange::new(start, end.max(start))
    }

    /// The offset of `position`. A client may name a column past a line's
    /// end, or a line past the last, which LSP reads as the end of the
    /// line or the text; one inside a character is its start.
    pub(crate) fn offset(&self, position: Position) -> TextSize {
        let at = LineCol {
            line: position.line,
            col: position.character,
        };
        self.index.clamped_offset(&self.text, at, self.unit)
    }

    /// `edit` as the protocol writes it.
    pub(crate) fn edit(&self, edit: nomi_ide::Edit) -> TextEdit {
        TextEdit::new(self.range(edit.range), edit.text)
    }
}

/// The files an answer names, by URI and text, each read through a
/// snapshot once for positions in it however often the answer names it.
pub(crate) struct Files<'s> {
    snapshot: &'s Snapshot,
    unit: ColumnUnit,
    /// None for a file that can't be read or named.
    read: HashMap<PathBuf, Option<(Url, Text<'static>)>>,
}

impl<'s> Files<'s> {
    pub(crate) fn new(snapshot: &'s Snapshot, unit: ColumnUnit) -> Files<'s> {
        Files {
            snapshot,
            unit,
            read: HashMap::new(),
        }
    }

    /// The URI and text of the file at `path`, where it can be read.
    pub(crate) fn get(&mut self, path: &Path) -> Option<&(Url, Text<'static>)> {
        if !self.read.contains_key(path) {
            let text = self.snapshot.read(path).ok();
            let file = (text.zip(Url::from_file_path(path).ok()))
                .map(|(text, uri)| (uri, Text::new(text, self.unit)));
            self.read.insert(path.to_owned(), file);
        }
        self.read[path].as_ref()
    }

    /// `location` as the protocol names it.
    pub(crate) fn location(&mut self, location: &nomi_ide::Location) -> Option<Location> {
        let (uri, text) = self.get(&location.path)?;
        Some(Location::new(uri.clone(), text.range(location.range)))
    }

    /// Each file's edits as the protocol writes them, made by `edit`, in
    /// order; a file that can't be read is left out.
    pub(crate) fn edits<E>(
        &mut self,
        files: impl IntoIterator<Item = (PathBuf, Vec<E>)>,
        edit: impl Fn(&Text<'static>, E) -> TextEdit,
    ) -> Vec<(Url, Vec<TextEdit>)> {
        (files.into_iter())
            .filter_map(|(path, edits)| {
                let (uri, text) = self.get(&path)?;
                Some((
                    uri.clone(),
                    edits.into_iter().map(|e| edit(text, e)).collect(),
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_in_the_negotiated_unit() {
        let source = "a😀b\nc";
        for (unit, column) in [(ColumnUnit::Utf16, 3), (ColumnUnit::Utf8, 5)] {
            let text = Text::new(source, unit);
            let b = TextSize::new(5);
            assert_eq!(text.position(b), Position::new(0, column));
            assert_eq!(text.offset(text.position(b)), b);
            // Past the line's end is its end; past the text, the text's.
            assert_eq!(usize::from(text.offset(Position::new(0, 99))), 6);
            assert_eq!(usize::from(text.offset(Position::new(9, 0))), 8);
        }
        let range = TextRange::new(0.into(), 1.into());
        assert_eq!(
            Text::new(source, ColumnUnit::Utf16)
                .range(range)
                .end
                .character,
            1
        );
    }
}
