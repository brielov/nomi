//! The one structured diagnostic model (architecture §4). Phases build
//! these and never format strings; renderers for the terminal,
//! `--report=json` and LSP come later.

pub mod doc;

use std::borrow::Cow;
use std::fmt;

use text_size::TextRange;

use crate::FileId;
pub use doc::{Doc, Snippet};

/// A range in one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub range: TextRange,
}

impl Span {
    pub const fn new(file: FileId, range: TextRange) -> Span {
        Span { file, range }
    }
}

/// A stable name for a kind of diagnostic, such as `syntax/unfinished-let`:
/// lower-case ASCII words joined by `-`, grouped by `/`. Codes are part of
/// the interface, for suppressions and docs, so one never changes meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(&'static str);

impl Code {
    /// Panics, at compile time in a `const`, if `code` is malformed.
    pub const fn new(code: &'static str) -> Code {
        assert!(is_valid_code(code), "malformed diagnostic code");
        Code(code)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

const fn is_valid_code(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut previous = b'/';
    while i < bytes.len() {
        let b = bytes[i];
        let word = b.is_ascii_lowercase() || b.is_ascii_digit();
        let joiner = b == b'-' || b == b'/';
        if !(word || joiner) || (joiner && (previous == b'-' || previous == b'/')) {
            return false;
        }
        previous = b;
        i += 1;
    }
    !bytes.is_empty() && previous != b'-' && previous != b'/'
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Shown faded in an editor, such as unused code.
    Hint,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    /// Upstream's title where upstream has the diagnostic, such as
    /// `UNFINISHED LET`.
    pub title: Cow<'static, str>,
    pub severity: Severity,
    pub primary: Span,
    pub secondary: Vec<Label>,
    pub message: Doc,
    pub fixes: Vec<Fix>,
}

/// A secondary span with an optional note on what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub note: Doc,
}

/// A suggested change: edits applied together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    /// Sorted by span, none overlapping.
    edits: Vec<Edit>,
}

/// Replaces `span` with `insert`; an empty span inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub insert: String,
}

/// Two edits of one fix touch the same text, so their result would depend
/// on the order they were applied in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlappingEdits {
    pub first: Edit,
    pub second: Edit,
}

impl Diagnostic {
    pub fn new(
        code: Code,
        severity: Severity,
        title: impl Into<Cow<'static, str>>,
        primary: Span,
        message: Doc,
    ) -> Diagnostic {
        Diagnostic {
            code,
            title: title.into(),
            severity,
            primary,
            secondary: Vec::new(),
            message,
            fixes: Vec::new(),
        }
    }

    pub fn error(
        code: Code,
        title: impl Into<Cow<'static, str>>,
        primary: Span,
        message: Doc,
    ) -> Diagnostic {
        Diagnostic::new(code, Severity::Error, title, primary, message)
    }

    pub fn with_label(mut self, span: Span, note: Doc) -> Diagnostic {
        self.secondary.push(Label { span, note });
        self
    }

    pub fn with_fix(mut self, fix: Fix) -> Diagnostic {
        self.fixes.push(fix);
        self
    }
}

impl Fix {
    /// Sorts the edits; fails if two overlap. Edits that only touch, one
    /// ending where the next starts, are fine unless both are insertions
    /// at the same point.
    pub fn new(title: impl Into<String>, mut edits: Vec<Edit>) -> Result<Fix, OverlappingEdits> {
        edits.sort_by_key(|e| (e.span.file, e.span.range.start(), e.span.range.end()));
        for pair in edits.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let same_file = a.span.file == b.span.file;
            let overlap = a.span.range.end() > b.span.range.start()
                || (a.span.range.is_empty()
                    && b.span.range.is_empty()
                    && a.span.range == b.span.range);
            if same_file && overlap {
                return Err(OverlappingEdits {
                    first: a.clone(),
                    second: b.clone(),
                });
            }
        }
        Ok(Fix {
            title: title.into(),
            edits,
        })
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    /// `text` with this fix's edits to `file` applied. Panics if an edit
    /// falls outside `text` or inside a character.
    pub fn apply(&self, file: FileId, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut done = 0;
        for edit in self.edits.iter().filter(|e| e.span.file == file) {
            let range = edit.span.range;
            out.push_str(&text[done..usize::from(range.start())]);
            out.push_str(&edit.insert);
            done = range.end().into();
        }
        out.push_str(&text[done..]);
        out
    }
}

impl Edit {
    pub fn new(span: Span, insert: impl Into<String>) -> Edit {
        Edit {
            span,
            insert: insert.into(),
        }
    }
}

impl fmt::Display for OverlappingEdits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "edits at {:?} and {:?} overlap",
            self.first.span.range, self.second.span.range
        )
    }
}

impl std::error::Error for OverlappingEdits {}
