//! Lays documents out as upstream does: a port of `renderPretty 1 80` from
//! ansi-wl-pprint (0.6.8 and later), which `Reporting/Doc.hs` prints every
//! report with, and of `displayS` and `displayIO` for its output.
//!
//! A group goes on one line if it and whatever follows it up to the next
//! newline fit in what is left of the line; otherwise its line breaks
//! become newlines. With a ribbon of 1 the ribbon never binds before the
//! width does, so only the width is checked.

use std::fmt::Write as _;

use super::doc::{Color, Doc, Line};
use crate::Compat;

/// The width upstream lays everything out at.
pub const WIDTH: usize = 80;

/// How styles show: not at all, as `P.plain` leaves them for output that
/// isn't a terminal, or as ANSI escapes, as `displayIO` writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Styles {
    Plain,
    Ansi,
}

/// `doc` laid out at `width` columns, with links to `compat`'s docs.
/// Its snippets must have been put in with `code::expand`.
pub fn render(doc: &Doc, compat: Compat, width: usize, styles: Styles) -> String {
    let output = match styles {
        Styles::Plain => Output::Plain,
        Styles::Ansi => Output::Ansi,
    };
    Renderer::new(compat, width, output, doc).run().out
}

/// A run of text in one style: how `D.encode` gives a message to
/// `--report=json`, cut at every change of style, so a chunk may be empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub(crate) text: String,
    pub(crate) style: Style,
}

/// `doc` laid out as `D.encode` lays it out, at [`WIDTH`] columns, in
/// chunks: the one before the first change of style, then one after each.
pub(crate) fn chunks(doc: &Doc, compat: Compat) -> Vec<Chunk> {
    let mut renderer = Renderer::new(compat, WIDTH, Output::Chunks, doc).run();
    let last = renderer.take_chunk();
    renderer.chunks.push(last);
    renderer.chunks
}

/// The styles in effect, which `RestoreFormat` brings back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) color: Option<Color>,
    pub(crate) underline: bool,
}

/// What a change of style does: nothing, an ANSI escape, or the end of a
/// chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Plain,
    Ansi,
    Chunks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// In a group on one line, as `flatten` leaves it.
    Flat,
    Break,
}

/// What is left to lay out, innermost last.
#[derive(Clone, Copy, Debug)]
enum Cmd<'d> {
    Doc {
        indent: usize,
        mode: Mode,
        doc: &'d Doc,
    },
    /// The end of a `fill` that started at column `start`.
    Pad { start: usize, width: usize },
    /// The end of a style.
    Restore(Style),
}

impl<'d> Cmd<'d> {
    fn doc(indent: usize, mode: Mode, doc: &'d Doc) -> Cmd<'d> {
        Cmd::Doc { indent, mode, doc }
    }
}

struct Renderer<'d> {
    width: usize,
    compat: Compat,
    output: Output,
    out: String,
    chunks: Vec<Chunk>,
    column: usize,
    style: Style,
    stack: Vec<Cmd<'d>>,
}

impl<'d> Renderer<'d> {
    fn new(compat: Compat, width: usize, output: Output, doc: &'d Doc) -> Renderer<'d> {
        Renderer {
            width,
            compat,
            output,
            out: String::new(),
            chunks: Vec::new(),
            column: 0,
            style: Style::default(),
            stack: vec![Cmd::doc(0, Mode::Break, doc)],
        }
    }

    fn run(mut self) -> Self {
        while let Some(cmd) = self.stack.pop() {
            let (indent, mode, doc) = match cmd {
                Cmd::Doc { indent, mode, doc } => (indent, mode, doc),
                Cmd::Pad { start, width } => {
                    let spaces = padding(start, self.column, width);
                    self.push_text(&" ".repeat(spaces));
                    continue;
                }
                Cmd::Restore(style) => {
                    self.restyle(style, &restore_codes(style));
                    continue;
                }
            };
            let push = |stack: &mut Vec<Cmd<'d>>, indent, mode, doc| {
                stack.push(Cmd::doc(indent, mode, doc));
            };
            match doc {
                Doc::Empty => {}
                Doc::Text(text) => self.push_text(text),
                Doc::Link(page) => self.push_text(&link(self.compat, page)),
                Doc::Line(line) => match (mode, line) {
                    (Mode::Flat, Line::Space) => self.push_text(" "),
                    (Mode::Flat, Line::Empty) => {}
                    // A flat group never holds a hard line, since it
                    // wouldn't fit.
                    (Mode::Break, _) | (Mode::Flat, Line::Hard) => {
                        self.out.push('\n');
                        self.out.push_str(&" ".repeat(indent));
                        self.column = indent;
                    }
                },
                Doc::Append(a, b) => {
                    push(&mut self.stack, indent, mode, b);
                    push(&mut self.stack, indent, mode, a);
                }
                Doc::Nest(width, d) => push(&mut self.stack, indent + width, mode, d),
                Doc::Align(d) => push(&mut self.stack, self.column, mode, d),
                Doc::Group(d) => {
                    let mode = if mode == Mode::Flat || self.fits(indent, d) {
                        Mode::Flat
                    } else {
                        Mode::Break
                    };
                    push(&mut self.stack, indent, mode, d);
                }
                Doc::Fill(width, d) => {
                    self.stack.push(Cmd::Pad {
                        start: self.column,
                        width: *width,
                    });
                    push(&mut self.stack, indent, mode, d);
                }
                Doc::Color(color, d) => self.styled(indent, mode, d, |style| {
                    style.color = Some(*color);
                    color_code(*color)
                }),
                Doc::Underline(d) => self.styled(indent, mode, d, |style| {
                    style.underline = true;
                    4
                }),
                Doc::Snippet(_) | Doc::Pair(_) => panic!("{UNEXPANDED}"),
            }
        }
        self
    }

    /// Lays out `doc` with a style `set` adds, then restores the style.
    fn styled(&mut self, indent: usize, mode: Mode, doc: &'d Doc, set: impl Fn(&mut Style) -> u8) {
        if self.output != Output::Plain {
            self.stack.push(Cmd::Restore(self.style));
            let mut style = self.style;
            let code = set(&mut style);
            self.restyle(style, &[code]);
        }
        self.stack.push(Cmd::doc(indent, mode, doc));
    }

    fn push_text(&mut self, text: &str) {
        self.out.push_str(text);
        self.column += text.chars().count();
    }

    /// An `SGR` to `style` by `codes`: `setSGRCode`, or the end of a chunk.
    fn restyle(&mut self, style: Style, codes: &[u8]) {
        match self.output {
            Output::Plain => {}
            Output::Ansi => {
                self.out.push_str("\x1b[");
                for (i, code) in codes.iter().enumerate() {
                    let sep = if i == 0 { "" } else { ";" };
                    // Writing to a `String` can't fail.
                    let _ = write!(self.out, "{sep}{code}");
                }
                self.out.push('m');
            }
            Output::Chunks => {
                let chunk = self.take_chunk();
                self.chunks.push(chunk);
            }
        }
        self.style = style;
    }

    /// The text since the last change of style, in the style in effect.
    fn take_chunk(&mut self) -> Chunk {
        Chunk {
            text: std::mem::take(&mut self.out),
            style: self.style,
        }
    }

    /// `fits`: whether `doc` on one line, then what is left up to the next
    /// newline, stays within the width.
    fn fits(&self, indent: usize, doc: &'d Doc) -> bool {
        let mut column = self.column;
        let mut local = vec![Cmd::doc(indent, Mode::Flat, doc)];
        let mut rest = self.stack.iter().rev();
        loop {
            if column > self.width {
                return false;
            }
            // Groups after this one are taken broken: whether they would
            // go on one line or not, the text before their first line
            // break is the same, and the check ends there.
            let Some(cmd) = local.pop().or_else(|| rest.next().copied()) else {
                return true;
            };
            let (indent, mode, doc) = match cmd {
                Cmd::Doc { indent, mode, doc } => (indent, mode, doc),
                Cmd::Pad { start, width } => {
                    column += padding(start, column, width);
                    continue;
                }
                Cmd::Restore(_) => continue,
            };
            match doc {
                Doc::Empty => {}
                Doc::Text(text) => column += text.chars().count(),
                Doc::Link(page) => column += link(self.compat, page).chars().count(),
                Doc::Line(line) => match (mode, line) {
                    (Mode::Flat, Line::Space) => column += 1,
                    (Mode::Flat, Line::Empty) => {}
                    (Mode::Flat, Line::Hard) => return false,
                    (Mode::Break, _) => return true,
                },
                Doc::Append(a, b) => {
                    local.push(Cmd::doc(indent, mode, b));
                    local.push(Cmd::doc(indent, mode, a));
                }
                Doc::Nest(width, d) => local.push(Cmd::doc(indent + width, mode, d)),
                Doc::Fill(width, d) => {
                    local.push(Cmd::Pad {
                        start: column,
                        width: *width,
                    });
                    local.push(Cmd::doc(indent, mode, d));
                }
                Doc::Align(d) | Doc::Group(d) | Doc::Color(_, d) | Doc::Underline(d) => {
                    local.push(Cmd::doc(indent, mode, d));
                }
                Doc::Snippet(_) | Doc::Pair(_) => panic!("{UNEXPANDED}"),
            }
        }
    }
}

const UNEXPANDED: &str = "a snippet laid out before `code::expand` put its lines in";

/// `fill`'s spaces: up to `width` columns past `start`, where `column`
/// is, which may be before `start` after a newline.
fn padding(start: usize, column: usize, width: usize) -> usize {
    let used = column as isize - start as isize;
    (width as isize - used).max(0) as usize
}

/// `makeLink`.
fn link(compat: Compat, page: &str) -> String {
    format!("<https://elm-lang.org/{compat}/{page}>")
}

/// `SetColor Foreground`: vivid colors are the bright ones.
fn color_code(color: Color) -> u8 {
    let (base, index) = match color {
        Color::Black => (90, 0),
        Color::Red => (90, 1),
        Color::Green => (90, 2),
        Color::Yellow => (90, 3),
        Color::Blue => (90, 4),
        Color::Magenta => (90, 5),
        Color::Cyan => (90, 6),
        Color::White => (90, 7),
        Color::DullBlack => (30, 0),
        Color::DullRed => (30, 1),
        Color::DullGreen => (30, 2),
        Color::DullYellow => (30, 3),
        Color::DullBlue => (30, 4),
        Color::DullMagenta => (30, 5),
        Color::DullCyan => (30, 6),
        Color::DullWhite => (30, 7),
    };
    base + index
}

/// `RestoreFormat`: a reset, then the color and underlining in effect,
/// in that order. Upstream restores the intensity between the two, but
/// no report sets it.
fn restore_codes(style: Style) -> Vec<u8> {
    let mut codes = vec![0];
    codes.extend(style.color.map(color_code));
    if style.underline {
        codes.push(4);
    }
    codes
}

#[cfg(test)]
mod tests;
