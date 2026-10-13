//! The layouts elm-format builds its output from, a port of
//! `ElmFormat/Render/ElmStructure.hs`. Each joins its parts on one line
//! when they are all single lines and nothing forces a break, and stacks
//! them otherwise.

use super::block::{Block, Line};
use crate::tree::{ApplicationMultiline, ForceMultiline, Multiline};

/// `lines` with `separator` between each two.
pub(super) fn intersperse(
    lines: impl IntoIterator<Item = Line>,
    separator: impl Fn() -> Line,
) -> Line {
    let mut row = Line::default();
    for (i, line) in lines.into_iter().enumerate() {
        if i > 0 {
            row.push(separator());
        }
        row.push(line);
    }
    row
}

/// `items` with `separator()` between each two.
pub(super) fn interleave<T>(
    items: impl IntoIterator<Item = T>,
    mut separator: impl FnMut() -> T,
) -> impl Iterator<Item = T> {
    items
        .into_iter()
        .enumerate()
        .flat_map(move |(i, item)| (i > 0).then(&mut separator).into_iter().chain([item]))
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
    force: ForceMultiline,
    first: Block,
    rest: Vec<Block>,
    join: impl Fn(Vec<Line>) -> Line,
    stack: impl FnOnce(Block, Vec<Block>) -> Block,
) -> Block {
    if !force.0 && first.is_line() && all_lines(&rest) {
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
pub(super) fn space_sep_or_stack(first: Block, rest: Vec<Block>) -> Block {
    forceable_space_sep_or_stack(ForceMultiline(false), first, rest)
}

pub(super) fn forceable_space_sep_or_stack(
    force: ForceMultiline,
    first: Block,
    rest: Vec<Block>,
) -> Block {
    joined_or(
        force,
        first,
        rest,
        |parts| intersperse(parts, Line::space),
        |first, rest| Block::stack1(std::iter::once(first).chain(rest)),
    )
}

/// As `forceable_space_sep_or_stack`, without the spaces.
pub(super) fn forceable_row_or_stack(
    force: ForceMultiline,
    first: Block,
    rest: Vec<Block>,
) -> Block {
    joined_or(force, first, rest, Line::row, |first, rest| {
        Block::stack1(std::iter::once(first).chain(rest))
    })
}

/// As `forceable_space_sep_or_stack`, from one list. Panics on an empty
/// one, as upstream errors.
pub(super) fn forceable_space_sep_or_stack1(force: ForceMultiline, blocks: Vec<Block>) -> Block {
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
pub(super) fn space_sep_or_indented(first: Block, rest: Vec<Block>) -> Block {
    forceable_space_sep_or_indented(ForceMultiline(false), first, rest)
}

pub(super) fn forceable_space_sep_or_indented(
    force: ForceMultiline,
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
pub(super) fn space_sep_or_prefix(op: Block, rest: Block) -> Block {
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
pub(super) fn prefix_or_indented(a: Block, b: Block) -> Block {
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
pub(super) fn equals_pair(symbol: &str, force: ForceMultiline, left: Block, right: Block) -> Block {
    match (left, right) {
        (Block::SingleLine(left), Block::SingleLine(right)) if !force.0 => {
            Block::line(Line::row([
                left,
                Line::space(),
                Line::text(symbol),
                Line::space(),
                right,
            ]))
        }
        (Block::SingleLine(left), Block::MustBreak(right)) => Block::must_break(Line::row([
            left,
            Line::space(),
            Line::text(symbol),
            Line::space(),
            right,
        ])),
        (Block::SingleLine(left), right) => Block::stack1([
            Block::line(Line::row([left, Line::space(), Line::text(symbol)])),
            right.indent(),
        ]),
        (left, right) => Block::stack1([
            left,
            Block::line(Line::text(symbol)).indent(),
            right.indent(),
        ]),
    }
}

/// What a definition gives its name: a value, which goes below the `=`
/// whenever it can, or a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DefinitionKind {
    /// `name = value`.
    Value,
    /// `name : type`.
    Annotation,
}

/// An `equals_pair` whose left side is an application.
pub(super) fn definition(
    kind: DefinitionKind,
    first: Block,
    rest: Vec<Block>,
    right: Block,
) -> Block {
    let left = application(
        ApplicationMultiline::JoinFirst(Multiline::JoinAll),
        first,
        rest,
    );
    match kind {
        DefinitionKind::Value => equals_pair("=", ForceMultiline(true), left, right),
        DefinitionKind::Annotation => equals_pair(":", ForceMultiline(false), left, right),
    }
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
pub(super) fn application(
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

/// What a `group` stands between, and whether spaces pad its inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Delimiters {
    left: &'static str,
    right: &'static str,
    padded: bool,
}

impl Delimiters {
    /// `( a, b )`.
    pub(super) const PADDED_PARENS: Delimiters = Delimiters {
        left: "(",
        right: ")",
        padded: true,
    };
    /// `[ a, b ]`.
    pub(super) const PADDED_BRACKETS: Delimiters = Delimiters {
        left: "[",
        right: "]",
        padded: true,
    };
    /// `{ a, b }`.
    pub(super) const PADDED_BRACES: Delimiters = Delimiters {
        left: "{",
        right: "}",
        padded: true,
    };
    /// `(a, b)`.
    pub(super) const PARENS: Delimiters = Delimiters {
        left: "(",
        right: ")",
        padded: false,
    };
    /// `a, b`.
    pub(super) const NONE: Delimiters = Delimiters {
        left: "",
        right: "",
        padded: false,
    };
}

/// `group(Delimiters::PADDED_PARENS, …)` formats as:
///
/// ```text
/// ()
///
/// ( child0 )
///
/// ( child0, child1, child2 )
///
/// ( child0
/// , child1
/// , child2
/// )
/// ```
pub(super) fn group(delimiters: Delimiters, force: ForceMultiline, children: Vec<Block>) -> Block {
    group_with_footer(delimiters, Vec::new(), force, children)
}

/// A `group` whose `footer` follows the children: after a space, or
/// stacked before the right delimiter. Upstream's `group'`, which joins
/// the footer like a child, `(x, {- c -})`, so a module line doesn't
/// parse; nomi deviates (`DECISIONS.md`).
pub(super) fn group_with_footer(
    delimiters: Delimiters,
    footer: Vec<Block>,
    force: ForceMultiline,
    children: Vec<Block>,
) -> Block {
    let Delimiters {
        left,
        right,
        padded,
    } = delimiters;
    let singles = all_lines(&children) && all_lines(&footer);
    if singles && children.is_empty() {
        return Block::line(Line::row(
            std::iter::once(Line::text(left))
                .chain(lines(footer))
                .chain([Line::text(right)]),
        ));
    }
    if singles && !force.0 {
        let mut row = Line::text(left);
        if padded {
            row.push(Line::space());
        }
        row.push(intersperse(lines(children), || {
            Line::row([Line::text(","), Line::space()])
        }));
        for line in lines(footer) {
            row.push(Line::space());
            row.push(line);
        }
        if padded {
            row.push(Line::space());
        }
        row.push(Line::text(right));
        return Block::line(row);
    }
    let mut children = children.into_iter();
    let Some(first) = children.next() else {
        // Upstream drops the footer here, and notes it may never happen.
        return Block::line(Line::row([Line::text(left), Line::text(right)]));
    };
    Block::stack1(
        std::iter::once(first.prefix(&Line::row([Line::text(left), Line::space()])))
            .chain(children.map(|c| c.prefix(&Line::row([Line::text(","), Line::space()]))))
            .chain(footer)
            .chain([Block::line(Line::text(right))]),
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
pub(super) fn extension_group_fields(
    multiline: ForceMultiline,
    base: Block,
    fields: Block,
) -> Block {
    match (base, fields) {
        (Block::SingleLine(base), Block::SingleLine(fields)) if !multiline.0 => {
            Block::line(intersperse(
                [Line::text("{"), base, fields, Line::text("}")],
                Line::space,
            ))
        }
        (base, fields) => Block::stack1([
            base.prefix(&Line::row([Line::text("{"), Line::space()])),
            fields.indent(),
            Block::line(Line::text("}")),
        ]),
    }
}

#[cfg(test)]
mod tests;
