//! Doc comments' Markdown: the Cheapskate fork elm-format 0.8.8 parses
//! it with (`elm-format-markdown/Cheapskate`), and upstream's
//! `ElmFormat/Render/Markdown.hs`, which prints it back.
//!
//! The port is faithful, quirks included, because a doc comment comes out
//! as upstream reads it: a `CommonMark` parser would see other blocks. Code
//! blocks carry a payload, `C`, which lowering fills with the Elm code it
//! finds in them, so rendering never parses.

mod blocks;
mod inlines;
mod render;
mod scanner;

pub(crate) use blocks::parse;
pub(crate) use render::render;

/// A block; Cheapskate's `Block`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block<C> {
    Para(Vec<Inline>),
    Header(usize, Vec<Inline>),
    Blockquote(Vec<Block<C>>),
    List {
        tight: bool,
        kind: ListType,
        items: Vec<Vec<Block<C>>>,
    },
    /// The code as written, and what lowering made of it.
    Code {
        attr: CodeAttr,
        text: String,
        code: C,
    },
    Html(String),
    HRule,
    References(Vec<Reference>),
    /// Each `@docs` line's names.
    ElmDocs(Vec<Vec<String>>),
}

/// A link reference definition, `[label]: url "title"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    pub(crate) label: String,
    pub(crate) url: String,
    /// Empty when there is none.
    pub(crate) title: String,
}

impl Reference {
    /// What upstream keeps of a line that doesn't parse as a reference.
    fn unparsed() -> Reference {
        let unknown = || "??".to_owned();
        Reference {
            label: unknown(),
            url: unknown(),
            title: unknown(),
        }
    }
}

/// A code block's language: the first word of a fenced block's info
/// string, and none for an indented block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeAttr {
    pub(crate) lang: String,
}

impl CodeAttr {
    /// Whether the block holds Elm: marked `elm`, or not at all.
    pub(crate) fn is_elm(&self) -> bool {
        self.lang == "elm" || self.lang.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListType {
    Bullet(char),
    Numbered(NumWrapper, i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumWrapper {
    PeriodFollowing,
    ParenFollowing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Inline {
    Str(String),
    Space,
    SoftBreak,
    LineBreak,
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Code(String),
    Link {
        text: Vec<Inline>,
        target: LinkTarget,
        /// Empty when there is none.
        title: String,
    },
    Image {
        text: Vec<Inline>,
        url: String,
        /// Empty when there is none.
        title: String,
    },
    Entity(String),
    RawHtml(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    Url(String),
    Ref(String),
}

/// `Data.Text`'s `strip`, with GHC's `isSpace`.
fn strip(text: &str) -> &str {
    text.trim_matches(nomi_base::ghc::is_space)
}

#[cfg(test)]
mod tests;
