//! Message documents: what a diagnostic says, before any layout.
//!
//! The operations are the ones Elm's reports use, so a renderer can lay
//! them out at 80 columns exactly as upstream does. Phases build documents
//! and never format strings. Source snippets and links to Elm's docs stay
//! symbolic: the renderer fills them in from the file's text and the
//! compatibility target's version.

use text_size::TextRange;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Doc {
    Empty,
    /// Never contains a newline; use [`Doc::line`].
    Text(String),
    Line(Line),
    Append(Box<Doc>, Box<Doc>),
    Nest(usize, Box<Doc>),
    Align(Box<Doc>),
    Group(Box<Doc>),
    /// The document, then spaces up to the given width from where it
    /// started, if it ended short of it.
    Fill(usize, Box<Doc>),
    Color(Color, Box<Doc>),
    Bold(Box<Doc>),
    Underline(Box<Doc>),
    /// Lines of the diagnostic's primary file.
    Snippet(Snippet),
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
    pub fn empty() -> Doc {
        Doc::Empty
    }

    /// Panics if `text` has a newline, since the layout could not see it.
    pub fn text(text: impl Into<String>) -> Doc {
        let text = text.into();
        assert!(!text.contains('\n'), "Doc::text with a newline: {text:?}");
        if text.is_empty() {
            Doc::Empty
        } else {
            Doc::Text(text)
        }
    }

    pub fn line() -> Doc {
        Doc::Line(Line::Hard)
    }

    pub fn softline() -> Doc {
        Doc::Line(Line::Space)
    }

    pub fn linebreak() -> Doc {
        Doc::Line(Line::Empty)
    }

    pub fn snippet(region: TextRange, highlight: TextRange) -> Doc {
        Doc::Snippet(Snippet { region, highlight })
    }

    pub fn link(page: impl Into<String>) -> Doc {
        Doc::Link(page.into())
    }

    pub fn append(self, other: Doc) -> Doc {
        match (self, other) {
            (Doc::Empty, other) => other,
            (this, Doc::Empty) => this,
            (this, other) => Doc::Append(Box::new(this), Box::new(other)),
        }
    }

    pub fn space(self, other: Doc) -> Doc {
        self.append(Doc::text(" ")).append(other)
    }

    pub fn nest(self, width: usize) -> Doc {
        Doc::Nest(width, Box::new(self))
    }

    pub fn align(self) -> Doc {
        Doc::Align(Box::new(self))
    }

    pub fn hang(self, width: usize) -> Doc {
        self.nest(width).align()
    }

    pub fn group(self) -> Doc {
        Doc::Group(Box::new(self))
    }

    /// Pads with spaces to `width` columns (`fill`).
    pub fn fill(self, width: usize) -> Doc {
        Doc::Fill(width, Box::new(self))
    }

    pub fn indent(self, width: usize) -> Doc {
        Doc::text(" ".repeat(width))
            .append(self.nest(width))
            .align()
    }

    pub fn color(self, color: Color) -> Doc {
        Doc::Color(color, Box::new(self))
    }

    pub fn bold(self) -> Doc {
        Doc::Bold(Box::new(self))
    }

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
            | Doc::Bold(d)
            | Doc::Underline(d) => d.write_plain(out),
            Doc::Snippet(_) => out.push_str("[snippet]"),
            Doc::Link(page) => {
                out.push('<');
                out.push_str(page);
                out.push('>');
            }
        }
    }
}

pub fn text(text: impl Into<String>) -> Doc {
    Doc::text(text)
}

/// Joins `docs` with `sep` between each pair.
fn join(docs: impl IntoIterator<Item = Doc>, sep: impl Fn() -> Doc) -> Doc {
    let mut docs = docs.into_iter();
    let Some(first) = docs.next() else {
        return Doc::empty();
    };
    docs.fold(first, |left, right| left.append(sep()).append(right))
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

/// `a`, `a or b`, `a, b, or c` for `conjunction` "or". Commas go after
/// the item, outside any styling it has, as in Elm's reports.
pub fn comma_sep(conjunction: &str, values: Vec<Doc>) -> Vec<Doc> {
    let count = values.len();
    let mut docs = Vec::with_capacity(count + 1);
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 && index + 1 == count {
            docs.push(text(conjunction));
        }
        docs.push(if count > 2 && index + 1 < count {
            value.append(text(","))
        } else {
            value
        });
    }
    docs
}

/// `Label: before <link> after`, wrapped.
pub fn link(label: &str, before: &str, page: &str, after: &str) -> Doc {
    let label = text(label).underline().append(text(":"));
    fill_sep(
        std::iter::once(label)
            .chain(words(before))
            .chain([Doc::link(page)])
            .chain(words(after)),
    )
}

/// `before <link> after`, wrapped.
pub fn reflow_link(before: &str, page: &str, after: &str) -> Doc {
    fill_sep(words(before).chain([Doc::link(page)]).chain(words(after)))
}

pub fn simple_note(message: &str) -> Doc {
    labelled("Note", words(message))
}

pub fn simple_hint(message: &str) -> Doc {
    hint(words(message))
}

pub fn hint(docs: impl IntoIterator<Item = Doc>) -> Doc {
    labelled("Hint", docs)
}

fn labelled(label: &str, docs: impl IntoIterator<Item = Doc>) -> Doc {
    fill_sep(std::iter::once(text(label).underline().append(text(":"))).chain(docs))
}

fn words(text: &str) -> impl Iterator<Item = Doc> + '_ {
    text.split_whitespace().map(Doc::text)
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
