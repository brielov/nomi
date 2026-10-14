//! What every report of a module's errors shares: the module's source,
//! the release, and how a node's range sheds the trivia nomi's nodes keep
//! in front, which the caller knows from the tree.

use nomi_base::diagnostic::Problem;
use nomi_base::diagnostic::code::{self, Source};
use nomi_base::diagnostic::doc::{self, Doc};
use nomi_base::{Compat, TextRange};

/// What a report needs around its error.
pub struct ReportContext<'a> {
    pub source: &'a Source<'a>,
    pub compat: Compat,
    /// A node's range without the trivia in front of it.
    pub trim: &'a dyn Fn(TextRange) -> TextRange,
}

impl std::fmt::Debug for ReportContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportContext")
            .field("compat", &self.compat)
            .finish_non_exhaustive()
    }
}

impl ReportContext<'_> {
    /// `Code.toSnippet`: the code at `range`, `highlight` underlined,
    /// between `before` and `after`.
    pub(crate) fn snippet(
        &self,
        range: TextRange,
        highlight: Option<TextRange>,
        before: Doc,
        after: Doc,
    ) -> Doc {
        doc::snippet(before, (self.trim)(range), highlight.map(self.trim), after)
    }

    /// `report` as a problem, placed where its trimmed range is.
    pub(crate) fn problem(&self, report: &Report) -> Problem {
        let region = self.source.region((self.trim)(report.range), self.compat);
        let message = code::expand(&report.message, self.source, self.compat);
        Problem::new(report.title, region, message)
    }
}

/// A report before its range is trimmed and placed.
pub(crate) struct Report {
    pub(crate) title: &'static str,
    pub(crate) range: TextRange,
    pub(crate) message: Doc,
}

impl Report {
    pub(crate) fn new(title: &'static str, range: TextRange, message: Doc) -> Report {
        Report {
            title,
            range,
            message,
        }
    }

    /// A `TYPE MISMATCH` at `range`.
    pub(crate) fn mismatch(range: TextRange, message: Doc) -> Report {
        Report::new("TYPE MISMATCH", range, message)
    }
}
