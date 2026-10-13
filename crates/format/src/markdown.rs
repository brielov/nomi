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
    /// Each reference's label, URL and title.
    References(Vec<(String, String, String)>),
    /// Each `@docs` line's names.
    ElmDocs(Vec<Vec<String>>),
}

/// A fenced code block's info string: its first word, and the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeAttr {
    pub(crate) lang: String,
    pub(crate) info: String,
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
    /// The text, the target and the title.
    Link(Vec<Inline>, LinkTarget, String),
    /// The text, the URL and the title.
    Image(Vec<Inline>, String, String),
    Entity(String),
    RawHtml(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    Url(String),
    Ref(String),
}

/// Haskell's `lines`: no last, empty line after a final newline.
fn lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// `Data.Text`'s `strip`, with GHC's `isSpace`.
fn strip(text: &str) -> &str {
    text.trim_matches(nomi_base::ghc::is_space)
}

#[cfg(test)]
mod tests;
