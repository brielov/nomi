//! The layouts elm-format builds its output from, a port of
//! `ElmFormat/Render/ElmStructure.hs`. Each joins its parts on one line
//! when they are all single lines and nothing forces a break, and stacks
//! them otherwise.

use crate::block::{Block, Line};
use crate::tree::{ApplicationMultiline, Multiline};

fn punc(text: &str) -> Line {
    Line::text(text)
}

/// `lines` with `separator` between each two.
fn intersperse(lines: impl IntoIterator<Item = Line>, separator: impl Fn() -> Line) -> Line {
    let mut row = Line::default();
    for (i, line) in lines.into_iter().enumerate() {
        if i > 0 {
            row.push(separator());
        }
        row.push(line);
    }
    row
}

fn all_lines(blocks: &[Block]) -> bool {
    blocks.iter().all(Block::is_line)
}

fn lines(blocks: Vec<Block>) -> Vec<Line> {
    Block::all_singles(blocks).unwrap_or_else(|_| unreachable!("checked to be single lines"))
}

/// Joins `first` and `rest` with spaces when nothing forces a break and
/// all are single lines, or stacks them. The first item is the parts, in
/// order.
fn joined_or(
    force: bool,
    first: Block,
    rest: Vec<Block>,
    join: impl Fn(Vec<Line>) -> Line,
    stack: impl FnOnce(Block, Vec<Block>) -> Block,
) -> Block {
    if !force && first.is_line() && all_lines(&rest) {
        let mut parts = lines(vec![first]);
        parts.extend(lines(rest));
        return Block::line(join(parts));
    }
    stack(first, rest)
}

/// ```text
/// first rest0 rest1
///
/// first
/// rest0
/// rest1
/// ```
pub(crate) fn space_sep_or_stack(first: Block, rest: Vec<Block>) -> Block {
    forceable_space_sep_or_stack(false, first, rest)
}

pub(crate) fn forceable_space_sep_or_stack(force: bool, first: Block, rest: Vec<Block>) -> Block {
    joined_or(
        force,
        first,
        rest,
        |parts| intersperse(parts, Line::space),
        |first, rest| Block::stack1(std::iter::once(first).chain(rest)),
    )
}

/// As `forceable_space_sep_or_stack`, without the spaces.
pub(crate) fn forceable_row_or_stack(force: bool, first: Block, rest: Vec<Block>) -> Block {
    joined_or(force, first, rest, Line::row, |first, rest| {
        Block::stack1(std::iter::once(first).chain(rest))
    })
}

/// As `forceable_space_sep_or_stack`, from one list. Panics on an empty
/// one, as upstream errors.
pub(crate) fn forceable_space_sep_or_stack1(force: bool, blocks: Vec<Block>) -> Block {
    let mut blocks = blocks.into_iter();
    let first = blocks
        .next()
        .expect("forceableSpaceSepOrStack1 with empty list");
    forceable_space_sep_or_stack(force, first, blocks.collect())
}

/// ```text
/// first rest0 rest1 rest2
///
/// first
///     rest0
///     rest1
///     rest2
/// ```
pub(crate) fn space_sep_or_indented(first: Block, rest: Vec<Block>) -> Block {
    forceable_space_sep_or_indented(false, first, rest)
}

pub(crate) fn forceable_space_sep_or_indented(
    force: bool,
    first: Block,
    rest: Vec<Block>,
) -> Block {
    joined_or(
        force,
        first,
        rest,
        |parts| intersperse(parts, Line::space),
        |first, rest| {
            Block::stack1(std::iter::once(first).chain(rest.into_iter().map(Block::indent)))
        },
    )
}

/// ```text
/// op rest
///
/// op rest1
///    rest2
///
/// opLong
///     rest
/// ```
pub(crate) fn space_sep_or_prefix(op: Block, rest: Block) -> Block {
    match (op, rest) {
        (Block::SingleLine(op), Block::SingleLine(rest)) => {
            Block::line(Line::row([op, Line::space(), rest]))
        }
        (Block::SingleLine(op), rest) if op.length(0) < 4 => {
            rest.prefix(&Line::row([op, Line::space()]))
        }
        (op, rest) => Block::stack1([op, rest.indent()]),
    }
}

/// ```text
/// a b
///
/// a
///     b
/// ```
pub(crate) fn prefix_or_indented(a: Block, b: Block) -> Block {
    match (a, b) {
        (Block::SingleLine(a), Block::SingleLine(b)) => {
            Block::line(Line::row([a, Line::space(), b]))
        }
        (Block::SingleLine(a), Block::MustBreak(b)) => {
            Block::must_break(Line::row([a, Line::space(), b]))
        }
        (a, b) => Block::stack1([a, b.indent()]),
    }
}

/// ```text
/// left = right
///
/// left =
///     right
///
/// left
///     =
///     right
/// ```
pub(crate) fn equals_pair(symbol: &str, force: bool, left: Block, right: Block) -> Block {
    match (left, right) {
        (Block::SingleLine(left), Block::SingleLine(right)) if !force => Block::line(Line::row([
            left,
            Line::space(),
            punc(symbol),
            Line::space(),
            right,
        ])),
        (Block::SingleLine(left), Block::MustBreak(right)) => Block::must_break(Line::row([
            left,
            Line::space(),
            punc(symbol),
            Line::space(),
            right,
        ])),
        (Block::SingleLine(left), right) => Block::stack1([
            Block::line(Line::row([left, Line::space(), punc(symbol)])),
            right.indent(),
        ]),
        (left, right) => Block::stack1([left, Block::line(punc(symbol)).indent(), right.indent()]),
    }
}

/// An `equals_pair` whose left side is an application.
pub(crate) fn definition(
    symbol: &str,
    force: bool,
    first: Block,
    rest: Vec<Block>,
    right: Block,
) -> Block {
    let left = application(
        ApplicationMultiline::JoinFirst(Multiline::JoinAll),
        first,
        rest,
    );
    equals_pair(symbol, force, left, right)
}

/// ```text
/// first rest0 rest1 rest2
///
/// first rest0
///     rest1
///     rest2
///
/// first
///     rest0
///     rest1
///     rest2
/// ```
pub(crate) fn application(
    multiline: ApplicationMultiline,
    first: Block,
    args: Vec<Block>,
) -> Block {
    let mut args = args.into_iter();
    let Some(arg0) = args.next() else {
        return first;
    };
    let rest: Vec<Block> = args.collect();
    match (multiline, first, arg0) {
        (
            ApplicationMultiline::JoinFirst(Multiline::JoinAll),
            Block::SingleLine(first),
            Block::SingleLine(arg0),
        ) if all_lines(&rest) => Block::line(intersperse(
            [first, arg0].into_iter().chain(lines(rest)),
            Line::space,
        )),
        (ApplicationMultiline::JoinFirst(_), Block::SingleLine(first), Block::SingleLine(arg0)) => {
            Block::stack1(
                std::iter::once(Block::line(Line::row([first, Line::space(), arg0])))
                    .chain(rest.into_iter().map(Block::indent)),
            )
        }
        (_, first, arg0) => Block::stack1(
            std::iter::once(first).chain(std::iter::once(arg0).chain(rest).map(Block::indent)),
        ),
    }
}

/// `group(true, "<", ";", ">", …)` formats as:
///
/// ```text
/// <>
///
/// < child0 >
///
/// < child0; child1; child2 >
///
/// < child0
/// ; child1
/// ; child2
/// >
/// ```
pub(crate) fn group(
    inner_spaces: bool,
    left: &str,
    separator: &str,
    right: &str,
    force: bool,
    children: Vec<Block>,
) -> Block {
    group_with_footer(
        inner_spaces,
        left,
        separator,
        Vec::new(),
        right,
        force,
        children,
    )
}

/// A `group` whose `footer` follows the children: after a space, or
/// stacked before `right`. Upstream's `group'`, which joins the footer
/// like a child, `(x, {- c -})`, so a module line doesn't parse; nomi
/// deviates (`DECISIONS.md`).
pub(crate) fn group_with_footer(
    inner_spaces: bool,
    left: &str,
    separator: &str,
    footer: Vec<Block>,
    right: &str,
    force: bool,
    children: Vec<Block>,
) -> Block {
    let singles = all_lines(&children) && all_lines(&footer);
    if singles && children.is_empty() {
        return Block::line(Line::row(
            std::iter::once(punc(left))
                .chain(lines(footer))
                .chain([punc(right)]),
        ));
    }
    if singles && !force {
        let mut row = punc(left);
        if inner_spaces {
            row.push(Line::space());
        }
        row.push(intersperse(lines(children), || {
            Line::row([punc(separator), Line::space()])
        }));
        for line in lines(footer) {
            row.push(Line::space());
            row.push(line);
        }
        if inner_spaces {
            row.push(Line::space());
        }
        row.push(punc(right));
        return Block::line(row);
    }
    let mut children = children.into_iter();
    let Some(first) = children.next() else {
        // Upstream drops the footer here, and notes it may never happen.
        return Block::line(Line::row([punc(left), punc(right)]));
    };
    Block::stack1(
        std::iter::once(first.prefix(&Line::row([punc(left), Line::space()])))
            .chain(children.map(|c| c.prefix(&Line::row([punc(separator), Line::space()]))))
            .chain(footer)
            .chain([Block::line(punc(right))]),
    )
}

/// ```text
/// { base | first, rest0 }
///
/// { base
///     | first
///     , rest0
/// }
/// ```
///
/// The fields come already joined, `|` and `,` in place. Upstream's
/// `extensionGroup'`.
pub(crate) fn extension_group_fields(multiline: bool, base: Block, fields: Block) -> Block {
    match (base, fields) {
        (Block::SingleLine(base), Block::SingleLine(fields)) if !multiline => Block::line(
            intersperse([punc("{"), base, fields, punc("}")], Line::space),
        ),
        (base, fields) => Block::stack1([
            base.prefix(&Line::row([punc("{"), Line::space()])),
            fields.indent(),
            Block::line(punc("}")),
        ]),
    }
}

#[cfg(test)]
mod tests;
