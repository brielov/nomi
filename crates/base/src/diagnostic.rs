//! Upstream's reports (architecture §4), which phases build rather than
//! format strings. [`code`] puts in the source lines their snippets
//! show, then [`render`] lays them out for the terminal and [`json`] for
//! `--report=json`.

pub mod code;
pub mod doc;
pub mod json;
pub mod render;

use std::borrow::Cow;

pub use doc::Doc;
use doc::{Color, text};

/// A problem that isn't in a module, such as a bad `elm.json` or a
/// package that can't be found: `Help.Report`'s `Report`, which titles a
/// message and may name the file it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub title: Cow<'static, str>,
    pub path: Option<String>,
    pub message: Doc,
}

impl Report {
    /// `Help.report`: a paragraph, then more blocks.
    pub fn new(
        title: impl Into<Cow<'static, str>>,
        path: Option<String>,
        start: &str,
        others: impl IntoIterator<Item = Doc>,
    ) -> Report {
        Report::doc(title, path, doc::reflow(start), others)
    }

    /// `Help.docReport`: a block, then more blocks.
    pub fn doc(
        title: impl Into<Cow<'static, str>>,
        path: Option<String>,
        start: Doc,
        others: impl IntoIterator<Item = Doc>,
    ) -> Report {
        Report {
            title: title.into(),
            path,
            message: doc::stack(std::iter::once(start).chain(others)),
        }
    }

    /// `reportToDoc`: the title in a bar 80 columns wide that ends with
    /// the path, the message, and a blank line.
    pub fn to_doc(&self) -> Doc {
        let title_width = self.title.chars().count();
        let dashes = |used: usize| "-".repeat(80usize.saturating_sub(used).max(1));
        let end = match &self.path {
            None => dashes(4 + title_width),
            Some(path) => format!("{} {path}", dashes(5 + title_width + path.chars().count())),
        };
        let bar =
            doc::hsep([text("--"), text(self.title.clone()), text(end)]).color(Color::DullCyan);
        doc::stack([bar, self.message.clone(), Doc::empty()])
    }
}

/// A problem in a module as an Elm release reports it: `Report.Report`,
/// which titles a message and says where it is. The region is kept as the
/// release keeps positions, which byte offsets can't carry: 0.19.1's wrap
/// at 16 bits, and its errors can point past the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub title: Cow<'static, str>,
    pub region: code::Region,
    pub message: Doc,
}

impl Problem {
    /// `title` with `message`, at `region`.
    pub fn new(title: impl Into<Cow<'static, str>>, region: code::Region, message: Doc) -> Problem {
        Problem {
            title: title.into(),
            region,
            message,
        }
    }

    /// `Error.reportToJson`.
    pub fn to_json(&self, compat: crate::Compat) -> json::Json {
        json::problem(&self.title, self.region, &self.message, compat)
    }
}
