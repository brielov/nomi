//! Lays documents out as upstream does: a port of `renderPretty 1 80` from
//! ansi-wl-pprint (0.6.8 and later), which `Reporting/Doc.hs` prints every
//! report with, and of `displayS` and `displayIO` for its output.
//!
//! A group goes on one line if it and whatever follows it up to the next
//! newline fit in what is left of the line; otherwise its line breaks
//! become newlines. With a ribbon of 1 the ribbon never binds before the
//! width does, so only the width is checked.

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
pub fn render(doc: &Doc, compat: Compat, width: usize, styles: Styles) -> String {
    let mut renderer = Renderer {
        width,
        compat,
        styles,
        out: String::new(),
        column: 0,
        style: Style::default(),
        stack: vec![Cmd::Doc {
            indent: 0,
            mode: Mode::Break,
            doc,
        }],
    };
    renderer.run();
    renderer.out
}

/// The styles in effect, which `RestoreFormat` brings back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    color: Option<Color>,
    bold: bool,
    underline: bool,
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

struct Renderer<'d> {
    width: usize,
    compat: Compat,
    styles: Styles,
    out: String,
    column: usize,
    style: Style,
    stack: Vec<Cmd<'d>>,
}

impl<'d> Renderer<'d> {
    fn run(&mut self) {
        while let Some(cmd) = self.stack.pop() {
            let (indent, mode, doc) = match cmd {
                Cmd::Doc { indent, mode, doc } => (indent, mode, doc),
                Cmd::Pad { start, width } => {
                    let spaces = padding(start, self.column, width);
                    self.push_text(&" ".repeat(spaces));
                    continue;
                }
                Cmd::Restore(style) => {
                    self.style = style;
                    self.escape(&restore_codes(style));
                    continue;
                }
            };
            let push = |stack: &mut Vec<Cmd<'d>>, indent, mode, doc| {
                stack.push(Cmd::Doc { indent, mode, doc });
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
                Doc::Bold(d) => self.styled(indent, mode, d, |style| {
                    style.bold = true;
                    1
                }),
                Doc::Underline(d) => self.styled(indent, mode, d, |style| {
                    style.underline = true;
                    4
                }),
                Doc::Snippet(_) => unimplemented!("snippets render from plan 4.4"),
            }
        }
    }

    /// Lays out `doc` with a style `set` adds, then restores the style.
    fn styled(&mut self, indent: usize, mode: Mode, doc: &'d Doc, set: impl Fn(&mut Style) -> u8) {
        if self.styles == Styles::Ansi {
            self.stack.push(Cmd::Restore(self.style));
            let code = set(&mut self.style);
            self.escape(&[code]);
        }
        self.stack.push(Cmd::Doc { indent, mode, doc });
    }

    fn push_text(&mut self, text: &str) {
        self.out.push_str(text);
        self.column += text.chars().count();
    }

    /// `setSGRCode`.
    fn escape(&mut self, codes: &[u8]) {
        if self.styles == Styles::Ansi {
            let codes: Vec<String> = codes.iter().map(u8::to_string).collect();
            self.out.push_str(&format!("\x1b[{}m", codes.join(";")));
        }
    }

    /// `fits`: whether `doc` on one line, then what is left up to the next
    /// newline, stays within the width.
    fn fits(&self, indent: usize, doc: &'d Doc) -> bool {
        let mut room = self.width as isize - self.column as isize;
        let mut column = self.column;
        let mut local = vec![Cmd::Doc {
            indent,
            mode: Mode::Flat,
            doc,
        }];
        let mut rest = self.stack.iter().rev();
        let take = |n: usize, room: &mut isize, column: &mut usize| {
            *room -= n as isize;
            *column += n;
        };
        loop {
            if room < 0 {
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
                    take(padding(start, column, width), &mut room, &mut column);
                    continue;
                }
                Cmd::Restore(_) => continue,
            };
            match doc {
                Doc::Empty => {}
                Doc::Text(text) => take(text.chars().count(), &mut room, &mut column),
                Doc::Link(page) => {
                    take(
                        link(self.compat, page).chars().count(),
                        &mut room,
                        &mut column,
                    );
                }
                Doc::Line(line) => match (mode, line) {
                    (Mode::Flat, Line::Space) => take(1, &mut room, &mut column),
                    (Mode::Flat, Line::Empty) => {}
                    (Mode::Flat, Line::Hard) => return false,
                    (Mode::Break, _) => return true,
                },
                Doc::Append(a, b) => {
                    local.push(Cmd::Doc {
                        indent,
                        mode,
                        doc: b,
                    });
                    local.push(Cmd::Doc {
                        indent,
                        mode,
                        doc: a,
                    });
                }
                Doc::Nest(width, d) => local.push(Cmd::Doc {
                    indent: indent + width,
                    mode,
                    doc: d,
                }),
                Doc::Fill(width, d) => {
                    local.push(Cmd::Pad {
                        start: column,
                        width: *width,
                    });
                    local.push(Cmd::Doc {
                        indent,
                        mode,
                        doc: d,
                    });
                }
                Doc::Align(d)
                | Doc::Group(d)
                | Doc::Color(_, d)
                | Doc::Bold(d)
                | Doc::Underline(d) => local.push(Cmd::Doc {
                    indent,
                    mode,
                    doc: d,
                }),
                Doc::Snippet(_) => unimplemented!("snippets render from plan 4.4"),
            }
        }
    }
}

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

/// `RestoreFormat`: a reset, then the color, intensity and underlining
/// in effect, in that order.
fn restore_codes(style: Style) -> Vec<u8> {
    let mut codes = vec![0];
    codes.extend(style.color.map(color_code));
    if style.bold {
        codes.push(1);
    }
    if style.underline {
        codes.push(4);
    }
    codes
}

#[cfg(test)]
mod tests;
