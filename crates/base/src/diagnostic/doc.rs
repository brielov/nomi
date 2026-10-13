//! Message documents: what a diagnostic says, before any layout.
//!
//! The operations are the ones Elm's reports use, so a renderer can lay
//! them out at 80 columns exactly as upstream does. Phases build documents
//! and never format strings. Source snippets and links to Elm's docs stay
//! symbolic: the renderer fills them in from the file's text and the
//! compatibility target's version.

use text_size::TextRange;

/// A document, as ansi-wl-pprint's `Doc` builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Doc {
    Empty,
    /// Never contains a newline; use [`Doc::line`].
    Text(String),
    Line(Line),
    /// One document, then the other.
    Append(Box<Doc>, Box<Doc>),
    /// The document with the lines it breaks onto indented by the width
    /// more.
    Nest(usize, Box<Doc>),
    /// The document with the lines it breaks onto indented to the column
    /// it starts at.
    Align(Box<Doc>),
    /// The document on one line if it fits, its line breaks as in a group
    /// on one line; otherwise as it is.
    Group(Box<Doc>),
    /// The document, then spaces up to the given width from where it
    /// started, if it ended short of it.
    Fill(usize, Box<Doc>),
    Color(Color, Box<Doc>),
    Underline(Box<Doc>),
    /// Lines of the diagnostic's primary file.
    Snippet(Snippet),
    /// Two places in the diagnostic's primary file.
    Pair(Box<Pair>),
    /// `<https://elm-lang.org/{version}/{page}>`, with the version taken
    /// from the compatibility target.
    Link(String),
}

/// A line break. In a group laid out on one line, `Space` and `Empty`
/// become what they are named after, and `Hard` keeps the group from
/// being laid out on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    /// Always a newline (`hardline`).
    Hard,
    /// A newline, or a space in a group on one line (`line`).
    Space,
    /// A newline, or nothing in a group on one line (`linebreak`).
    Empty,
}

/// The lines covering `region`, with `highlight` underlined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub region: TextRange,
    pub highlight: TextRange,
}

/// Two regions, shown on one line when they share a row and as two
/// snippets otherwise (`Code.toPair`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub first: TextRange,
    pub second: TextRange,
    pub one_line: OneLine,
    pub two_snippets: TwoSnippets,
}

/// What a [`Pair`] shows around the one line holding both regions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneLine {
    pub before: Doc,
    pub after: Doc,
}

/// What a [`Pair`] shows around the two snippets of its regions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TwoSnippets {
    pub before: Doc,
    pub between: Doc,
    pub after: Doc,
}

/// Elm's diagnostic colors. Upper-case names in `--report=json` are the
/// bright ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    Red,
    DullRed,
    Magenta,
    DullMagenta,
    Yellow,
    DullYellow,
    Green,
    DullGreen,
    Cyan,
    DullCyan,
    Blue,
    DullBlue,
    Black,
    DullBlack,
    White,
    DullWhite,
}

impl Doc {
    /// Nothing (`empty`).
    pub fn empty() -> Doc {
        Doc::Empty
    }

    /// A newline that is one even in a group on one line (`hardline`).
    pub fn line() -> Doc {
        Doc::Line(Line::Hard)
    }

    /// A newline, or a space in a group on one line (`line`).
    pub fn softline() -> Doc {
        Doc::Line(Line::Space)
    }

    /// A newline, or nothing in a group on one line (`linebreak`).
    pub fn linebreak() -> Doc {
        Doc::Line(Line::Empty)
    }

    /// The lines covering `region`, with `highlight` underlined.
    pub fn snippet(region: TextRange, highlight: TextRange) -> Doc {
        Doc::Snippet(Snippet { region, highlight })
    }

    /// A link to `page` of the compatibility target's docs.
    pub fn link(page: impl Into<String>) -> Doc {
        Doc::Link(page.into())
    }

    /// This document, then `other` (`<>`).
    pub fn append(self, other: Doc) -> Doc {
        match (self, other) {
            (Doc::Empty, other) => other,
            (this, Doc::Empty) => this,
            (this, other) => Doc::Append(Box::new(this), Box::new(other)),
        }
    }

    /// This document, a space and `other` (`<+>`).
    pub fn space(self, other: Doc) -> Doc {
        self.append(text(" ")).append(other)
    }

    /// The lines this document breaks onto, indented by `width` more
    /// (`nest`).
    pub fn nest(self, width: usize) -> Doc {
        Doc::Nest(width, Box::new(self))
    }

    /// The lines this document breaks onto, indented to the column it
    /// starts at (`align`).
    pub fn align(self) -> Doc {
        Doc::Align(Box::new(self))
    }

    /// The lines this document breaks onto, indented by `width` past the
    /// column it starts at (`hang`).
    pub fn hang(self, width: usize) -> Doc {
        self.nest(width).align()
    }

    /// This document on one line if it fits (`group`).
    pub fn group(self) -> Doc {
        Doc::Group(Box::new(self))
    }

    /// Pads with spaces to `width` columns (`fill`).
    pub fn fill(self, width: usize) -> Doc {
        Doc::Fill(width, Box::new(self))
    }

    /// Every line of this document, the first too, indented by `width`
    /// (`indent`).
    pub fn indent(self, width: usize) -> Doc {
        text(" ".repeat(width)).append(self.nest(width)).align()
    }

    /// This document in `color`.
    pub fn color(self, color: Color) -> Doc {
        Doc::Color(color, Box::new(self))
    }

    /// This document underlined.
    pub fn underline(self) -> Doc {
        Doc::Underline(Box::new(self))
    }

    /// The text with layout and styles dropped: line breaks are as in a
    /// group on one line, a hard one a space, and snippets are elided. For tests and logs, not for users.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        self.write_plain(&mut out);
        out
    }

    fn write_plain(&self, out: &mut String) {
        match self {
            Doc::Empty | Doc::Line(Line::Empty) => {}
            Doc::Text(text) => out.push_str(text),
            Doc::Line(_) => out.push(' '),
            Doc::Append(a, b) => {
                a.write_plain(out);
                b.write_plain(out);
            }
            Doc::Nest(_, d)
            | Doc::Align(d)
            | Doc::Group(d)
            | Doc::Fill(_, d)
            | Doc::Color(_, d)
            | Doc::Underline(d) => d.write_plain(out),
            Doc::Snippet(_) | Doc::Pair(_) => out.push_str("[snippet]"),
            Doc::Link(page) => {
                out.push('<');
                out.push_str(page);
                out.push('>');
            }
        }
    }
}

/// `text` as a document. Panics if it has a newline, since the layout
/// could not see it.
pub fn text(text: impl Into<String>) -> Doc {
    let text = text.into();
    assert!(
        !text.contains('\n'),
        "a document's text with a newline: {text:?}"
    );
    if text.is_empty() {
        Doc::Empty
    } else {
        Doc::Text(text)
    }
}

/// A string as a document, as Haskell's `OverloadedStrings` makes one.
impl From<&str> for Doc {
    fn from(s: &str) -> Doc {
        text(s)
    }
}

impl From<String> for Doc {
    fn from(s: String) -> Doc {
        text(s)
    }
}

/// Joins `docs` with `sep` between each pair.
fn join(docs: impl IntoIterator<Item = Doc>, sep: impl Fn() -> Doc) -> Doc {
    let mut docs = docs.into_iter();
    let Some(first) = docs.next() else {
        return Doc::empty();
    };
    docs.fold(first, |left, right| left.append(sep()).append(right))
}

/// `Code.toSnippet`: `before`, a blank line, the lines of `region` with
/// `highlight` (or else the whole region) marked, and `after`.
pub fn snippet(before: Doc, region: TextRange, highlight: Option<TextRange>, after: Doc) -> Doc {
    framed(
        before,
        Doc::snippet(region, highlight.unwrap_or(region)),
        after,
    )
}

/// `before`, a blank line, `code` and `after`, as snippets are shown.
pub(crate) fn framed(before: Doc, code: Doc, after: Doc) -> Doc {
    vcat([before, Doc::empty(), code, after])
}

/// `Code.toPair`.
pub fn pair(
    first: TextRange,
    second: TextRange,
    one_line: OneLine,
    two_snippets: TwoSnippets,
) -> Doc {
    Doc::Pair(Box::new(Pair {
        first,
        second,
        one_line,
        two_snippets,
    }))
}

/// Words separated by spaces that never break.
pub fn hsep(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, || text(" "))
}

/// One per line, or all run together in a group on one line, as
/// `vcat` does.
pub fn vcat(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, Doc::linebreak)
}

/// All on one line separated by spaces, or all on separate lines.
pub fn sep(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, Doc::softline).group()
}

/// All on one line, or all on separate lines.
pub fn cat(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, Doc::linebreak).group()
}

/// As many per line as fit, separated by spaces.
pub fn fill_sep(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, || Doc::softline().group())
}

/// A paragraph that wraps at word boundaries.
pub fn reflow(paragraph: &str) -> Doc {
    fill_sep(words(paragraph))
}

/// Paragraphs separated by a blank line.
pub fn stack(docs: impl IntoIterator<Item = Doc>) -> Doc {
    join(docs, || Doc::linebreak().append(Doc::linebreak()))
}

/// The word [`comma_sep`] puts before the last item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conjunction {
    And,
    Or,
}

impl Conjunction {
    /// The word itself.
    pub const fn as_str(self) -> &'static str {
        match self {
            Conjunction::And => "and",
            Conjunction::Or => "or",
        }
    }
}

/// `a`, `a or b`, `a, b, or c` for [`Conjunction::Or`] (`commaSep`).
/// Commas go after the item, outside any styling it has, as in Elm's
/// reports.
pub fn comma_sep(conjunction: Conjunction, values: Vec<Doc>) -> Vec<Doc> {
    let count = values.len();
    let mut docs = Vec::with_capacity(count + 1);
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 && index + 1 == count {
            docs.push(text(conjunction.as_str()));
        }
        docs.push(if count > 2 && index + 1 < count {
            value.append(text(","))
        } else {
            value
        });
    }
    docs
}

/// `D.cycle 4`: `names` in a loop drawn with box-drawing characters, or
/// ASCII on Windows, as upstream draws them there.
pub fn cycle(names: impl IntoIterator<Item = String>) -> Doc {
    let (top, line, mid, end) = if cfg!(windows) {
        ("+-----+", "|    ", "|     |", "+-<---+")
    } else {
        ("┌─────┐", "│    ", "│     ↓", "└─────┘")
    };
    let mut rows = vec![text(top)];
    for (i, name) in names.into_iter().enumerate() {
        if i > 0 {
            rows.push(text(mid));
        }
        rows.push(text(line).append(dull_yellow(name)));
    }
    rows.push(text(end));
    vcat(rows).indent(4)
}

/// `D.args`: `1 argument`, `2 arguments`.
pub fn args(n: usize) -> String {
    match n {
        1 => "1 argument".to_owned(),
        n => format!("{n} arguments"),
    }
}

/// `D.link`: `Label: before <link> after`, wrapped.
pub fn link(label: &str, before: &str, page: &str, after: &str) -> Doc {
    labelled(label, link_words(before, page, after))
}

/// `D.reflowLink`: `before <link> after`, wrapped.
pub fn reflow_link(before: &str, page: &str, after: &str) -> Doc {
    fill_sep(link_words(before, page, after))
}

/// The words of `before`, a link to `page`, and the words of `after`.
fn link_words<'a>(before: &'a str, page: &str, after: &'a str) -> impl Iterator<Item = Doc> + 'a {
    words(before).chain([Doc::link(page)]).chain(words(after))
}

/// `D.toSimpleNote`: `Note: message`, wrapped.
pub fn simple_note(message: &str) -> Doc {
    note(words(message))
}

/// `D.toFancyNote`: `Note:` and `docs`, wrapped.
pub fn note(docs: impl IntoIterator<Item = Doc>) -> Doc {
    labelled("Note", docs)
}

/// `D.intToOrdinal`: 1st, 2nd, 3rd, 4th, 11th, 0th.
pub fn ordinal(n: usize) -> String {
    let ending = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{n}{ending}")
}

/// `D.toSimpleHint`: `Hint: message`, wrapped.
pub fn simple_hint(message: &str) -> Doc {
    hint(words(message))
}

/// `D.toFancyHint`: `Hint:` and `docs`, wrapped.
pub fn hint(docs: impl IntoIterator<Item = Doc>) -> Doc {
    labelled("Hint", docs)
}

/// `label` underlined with a colon, then `docs`, wrapped, as notes,
/// hints and links start.
pub fn labelled(label: &str, docs: impl IntoIterator<Item = Doc>) -> Doc {
    fill_sep(std::iter::once(text(label).underline().append(text(":"))).chain(docs))
}

/// The words of `text`, as `D.reflow` splits a paragraph.
pub fn words(text: &str) -> impl Iterator<Item = Doc> + '_ {
    text.split_whitespace().map(self::text)
}

/// `doc` in vivid red (`D.red`), which styles even no text.
pub fn red(doc: impl Into<Doc>) -> Doc {
    doc.into().color(Color::Red)
}

/// `doc` in vivid green (`D.green`).
pub fn green(doc: impl Into<Doc>) -> Doc {
    doc.into().color(Color::Green)
}

/// `doc` in vivid cyan (`D.cyan`).
pub fn cyan(doc: impl Into<Doc>) -> Doc {
    doc.into().color(Color::Cyan)
}

/// `doc` in vivid blue (`D.blue`).
pub fn blue(doc: impl Into<Doc>) -> Doc {
    doc.into().color(Color::Blue)
}

/// `doc` in dull yellow (`D.dullyellow`).
pub fn dull_yellow(doc: impl Into<Doc>) -> Doc {
    doc.into().color(Color::DullYellow)
}

impl Color {
    /// The name `--report=json` uses.
    pub fn json_name(self) -> &'static str {
        match self {
            Color::Red => "RED",
            Color::DullRed => "red",
            Color::Magenta => "MAGENTA",
            Color::DullMagenta => "magenta",
            Color::Yellow => "YELLOW",
            Color::DullYellow => "yellow",
            Color::Green => "GREEN",
            Color::DullGreen => "green",
            Color::Cyan => "CYAN",
            Color::DullCyan => "cyan",
            Color::Blue => "BLUE",
            Color::DullBlue => "blue",
            Color::Black => "BLACK",
            Color::DullBlack => "black",
            Color::White => "WHITE",
            Color::DullWhite => "white",
        }
    }
}
