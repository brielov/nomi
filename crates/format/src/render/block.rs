//! elm-format's layout primitives, a port of `Box.hs`. Its `Box` is
//! `Block` here, so it doesn't shadow `std::boxed::Box`.
//!
//! A block is lines, and layout is structural: whether a construct fits on
//! one line depends only on whether its parts are single lines, never on
//! width. So a block is built once, rendered once, and the work is linear
//! in the output.

use nomi_base::ghc::is_space;

/// One piece of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Atom {
    Text(String),
    Space,
    /// Advances to the next multiple of four columns.
    Tab,
}

/// Exactly one line of text, spaces and tabs.
///
/// Upstream nests rows (`Row [Line]`). Nesting has no effect on the output,
/// where a row is a left fold over its pieces, so a line keeps them flat.
/// The pieces prefixed to it are kept apart, last first, so indenting
/// stays cheap however deep the nesting goes, and joining two lines moves
/// the shorter one's pieces into the longer, so each piece moves at most
/// a logarithmic number of times however rows nest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Line {
    /// Prefixed pieces, in reverse.
    before: Vec<Atom>,
    after: Vec<Atom>,
}

const SPACES_IN_TAB: usize = 4;

fn tab_length(column: usize) -> usize {
    SPACES_IN_TAB - column % SPACES_IN_TAB
}

impl Line {
    /// Upstream's `identifier`, `keyword`, `punc` and `literal`, which are
    /// all the same.
    pub(super) fn text(text: impl Into<String>) -> Line {
        Line {
            before: Vec::new(),
            after: vec![Atom::Text(text.into())],
        }
    }

    pub(super) fn space() -> Line {
        Line {
            before: Vec::new(),
            after: vec![Atom::Space],
        }
    }

    /// Joins lines into one.
    pub(super) fn row(lines: impl IntoIterator<Item = Line>) -> Line {
        let mut row = Line::default();
        for line in lines {
            row.push(line);
        }
        row
    }

    /// Appends `line`.
    pub(super) fn push(&mut self, mut line: Line) {
        if line.len() > self.len() {
            let prefix = std::mem::take(self);
            line.before.extend(prefix.after.into_iter().rev());
            line.before.extend(prefix.before);
            *self = line;
            return;
        }
        self.after.extend(line.before.into_iter().rev());
        self.after.extend(line.after);
    }

    /// How many pieces the line holds.
    fn len(&self) -> usize {
        self.before.len() + self.after.len()
    }

    /// Prepends `line`.
    fn prepend(&mut self, line: &Line) {
        self.before.extend(line.after.iter().rev().cloned());
        self.before.extend(line.before.iter().cloned());
    }

    fn atoms(&self) -> impl Iterator<Item = &Atom> {
        self.before.iter().rev().chain(&self.after)
    }

    /// The column the line ends at when it starts at `start`; upstream's
    /// `lineLength`. Text counts characters, as `Data.Text` does.
    pub(super) fn length(&self, start: usize) -> usize {
        self.atoms().fold(start, |column, atom| match atom {
            Atom::Text(text) => column + text.chars().count(),
            Atom::Space => column + 1,
            Atom::Tab => column + tab_length(column),
        })
    }

    /// Appends the line as it renders from column 0, without trailing
    /// whitespace, and a newline.
    fn render_into(&self, out: &mut String) {
        let start = out.len();
        let mut column = 0;
        for atom in self.atoms() {
            match atom {
                Atom::Text(text) => {
                    out.push_str(text);
                    column += text.chars().count();
                }
                Atom::Space => {
                    out.push(' ');
                    column += 1;
                }
                Atom::Tab => {
                    let n = tab_length(column);
                    out.extend(std::iter::repeat_n(' ', n));
                    column += n;
                }
            }
        }
        let kept = out[start..].trim_end_matches(is_space).len();
        out.truncate(start + kept);
        out.push('\n');
    }
}

/// Lines, at least one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Block {
    /// One line that can join others.
    SingleLine(Line),
    /// Two or more lines.
    Stack(Vec<Line>),
    /// One line that must stand alone; only `--` comments are.
    MustBreak(Line),
}

impl Block {
    pub(super) fn line(line: Line) -> Block {
        Block::SingleLine(line)
    }

    pub(super) fn must_break(line: Line) -> Block {
        Block::MustBreak(line)
    }

    pub(super) fn blank_line() -> Block {
        Block::line(Line::text(""))
    }

    fn into_lines(self) -> Vec<Line> {
        match self {
            Block::SingleLine(line) | Block::MustBreak(line) => vec![line],
            Block::Stack(lines) => lines,
        }
    }

    /// `self`'s lines, then `other`'s; upstream's `stack'`.
    pub(super) fn stack(self, other: Block) -> Block {
        let mut lines = self.into_lines();
        match other {
            Block::SingleLine(line) | Block::MustBreak(line) => lines.push(line),
            Block::Stack(more) => lines.extend(more),
        }
        Block::Stack(lines)
    }

    /// Stacks `rest` below `self`; upstream's `andThen rest self`.
    pub(super) fn and_then(self, rest: impl IntoIterator<Item = Block>) -> Block {
        rest.into_iter().fold(self, Block::stack)
    }

    /// Stacks blocks; one block comes back as it is. Panics on none, as
    /// upstream errors.
    pub(super) fn stack1(blocks: impl IntoIterator<Item = Block>) -> Block {
        let mut blocks = blocks.into_iter();
        let first = blocks.next().expect("stack1: empty structure");
        match blocks.next() {
            None => first,
            Some(second) => first.stack(second).and_then(blocks),
        }
    }

    fn map_lines(self, first: impl FnOnce(&mut Line), mut rest: impl FnMut(&mut Line)) -> Block {
        match self {
            Block::SingleLine(mut line) => {
                first(&mut line);
                Block::SingleLine(line)
            }
            Block::MustBreak(mut line) => {
                first(&mut line);
                Block::MustBreak(line)
            }
            Block::Stack(mut lines) => {
                let (head, tail) = lines.split_first_mut().expect("a stack has lines");
                first(head);
                tail.iter_mut().for_each(&mut rest);
                Block::Stack(lines)
            }
        }
    }

    /// A tab before every line.
    pub(super) fn indent(self) -> Block {
        let tab = Line {
            before: Vec::new(),
            after: vec![Atom::Tab],
        };
        self.map_lines(|l| l.prepend(&tab), |l| l.prepend(&tab))
    }

    /// `prefix` before the first line, and as many spaces as it is long
    /// before the others:
    ///
    /// ```text
    /// abcde        myPrefix abcde
    /// xyz    -->            xyz
    /// ```
    pub(super) fn prefix(self, prefix: &Line) -> Block {
        let padding = Line {
            before: Vec::new(),
            after: vec![Atom::Space; prefix.length(0)],
        };
        self.map_lines(|l| l.prepend(prefix), |l| l.prepend(&padding))
    }

    /// `suffix` after the last line. A single line comes back joinable,
    /// even from `MustBreak`, as upstream's does.
    pub(super) fn add_suffix(self, suffix: Line) -> Block {
        match self {
            Block::SingleLine(mut line) | Block::MustBreak(mut line) => {
                line.push(suffix);
                Block::SingleLine(line)
            }
            Block::Stack(mut lines) => {
                lines.last_mut().expect("a stack has lines").push(suffix);
                Block::Stack(lines)
            }
        }
    }

    /// The line, if this is a joinable single line; upstream's `isLine`.
    pub(super) fn into_line(self) -> Result<Line, Block> {
        match self {
            Block::SingleLine(line) => Ok(line),
            block => Err(block),
        }
    }

    pub(super) fn is_line(&self) -> bool {
        matches!(self, Block::SingleLine(_))
    }

    /// Every block's line, if they are all joinable single lines.
    pub(super) fn all_singles(blocks: Vec<Block>) -> Result<Vec<Line>, Vec<Block>> {
        if !blocks.iter().all(Block::is_line) {
            return Err(blocks);
        }
        Ok(blocks
            .into_iter()
            .map(|b| b.into_line().expect("checked above"))
            .collect())
    }

    /// Each line from column 0, without trailing whitespace, and ending in
    /// a newline.
    pub(super) fn render(&self) -> String {
        let mut out = String::new();
        match self {
            Block::SingleLine(line) | Block::MustBreak(line) => line.render_into(&mut out),
            Block::Stack(lines) => lines.iter().for_each(|l| l.render_into(&mut out)),
        }
        out
    }
}

#[cfg(test)]
mod tests;
