//! Rendering: the formatter tree as elm-format lays it out, a port of
//! `ElmFormat/Render/Box.hs`.
//!
//! Each function builds the block its upstream namesake does, from the
//! same parts in the same order, so the two can be read side by side.

mod block;
mod declarations;
mod expressions;
mod header;
mod literal;
mod patterns;
mod structure;
mod types;

use nomi_syntax::SyntaxKind;

use block::{Block, Line};
use structure::{
    extension_group_fields, forceable_row_or_stack, forceable_space_sep_or_indented,
    forceable_space_sep_or_stack, forceable_space_sep_or_stack1, intersperse,
    space_sep_or_indented, space_sep_or_prefix, space_sep_or_stack,
};

use crate::markdown;
use crate::tree::{
    C2, Comment, Docs, ElmCode, ForceMultiline, Lower, Module, Pair, SeparatorComments, Sequence,
    Symbol, TopLevelStructure, Upper, UpperRef,
};

/// `module` as elm-format prints it.
pub(crate) fn module(module: &Module) -> String {
    format_module(module, ModuleContext::File).map_or_else(String::new, |b| b.render())
}

/// Where a module is formatted, which decides its spacing and whether it
/// gets a module line it lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleContext {
    /// A file of its own, which gets the default module line if it has
    /// none.
    File,
    /// A code block in a doc comment.
    CodeBlock,
}

impl ModuleContext {
    /// The blank lines between top-level parts.
    fn spacing(self) -> usize {
        match self {
            ModuleContext::File => 2,
            ModuleContext::CodeBlock => 1,
        }
    }
}

/// `module`, as `context` lays it out; upstream's `formatModule`.
/// Nothing at all only for a module in a code block, which can have no
/// parts.
fn format_module(module: &Module, context: ModuleContext) -> Option<Block> {
    let spacing = context.spacing();
    let mut blocks = Vec::new();
    if !module.initial_comments.is_empty() {
        blocks.extend(module.initial_comments.iter().map(comment));
        blocks.extend([Block::blank_line(), Block::blank_line()]);
    }
    blocks.extend(header::module_header(module, context));
    let space_before_body = match module.body.first() {
        None => 0,
        Some(TopLevelStructure::BodyComment(_)) => spacing + 1,
        Some(_) => spacing,
    };
    blocks.extend(std::iter::repeat_with(Block::blank_line).take(space_before_body));
    blocks.extend(declarations::body(spacing, &module.body));
    (!blocks.is_empty()).then(|| Block::stack1(blocks))
}

/// A line of `text`.
fn text(text: impl Into<String>) -> Block {
    Block::line(Line::text(text))
}

/// `(inner)`; upstream's `parens`.
fn parens(inner: Block) -> Block {
    surround('(', ')', inner)
}

/// Where a construct stands, which decides whether it needs parentheses
/// in another; upstream's `SyntaxContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyntaxContext {
    /// Delimited, or a single token.
    SyntaxSeparated,
    /// Operands joined by operators.
    InfixSeparated,
    /// A function and its arguments.
    SpaceSeparated,
    /// What ends in an expression, like a lambda or `if`.
    AmbiguousEnd,
}

/// `block` in parentheses when, standing as `inner`, it needs them in
/// `outer`; upstream's `syntaxParens`.
fn syntax_parens(outer: SyntaxContext, (inner, block): (SyntaxContext, Block)) -> Block {
    use SyntaxContext::{AmbiguousEnd, InfixSeparated, SpaceSeparated};
    let needed = matches!(
        (inner, outer),
        (
            SpaceSeparated | InfixSeparated | AmbiguousEnd,
            SpaceSeparated
        ) | (InfixSeparated | AmbiguousEnd, InfixSeparated)
            | (InfixSeparated, AmbiguousEnd)
    );
    if needed { parens(block) } else { block }
}

/// `left` and `right` around `inner`; upstream's `surround`.
fn surround(left: char, right: char, inner: Block) -> Block {
    let (left, right) = (Line::text(left), Line::text(right));
    match inner {
        Block::SingleLine(line) => Block::line(Line::row([left, line, right])),
        inner => Block::stack1([inner.prefix(&left), Block::line(right)]),
    }
}

/// `()`, `[]` or `{}` with the comments inside; upstream's `formatUnit`.
fn unit(left: char, right: char, comments: &[Comment]) -> Block {
    match comments {
        [] => Block::line(Line::text(format!("{left}{right}"))),
        [Comment::Line(_), ..] if left == '{' => surround(
            left,
            right,
            Block::stack1(comments.iter().map(comment)).prefix(&Line::space()),
        ),
        _ => surround(
            left,
            right,
            match Block::all_singles(comments.iter().map(comment).collect()) {
                Ok(lines) => Block::line(intersperse(lines, Line::space)),
                Err(blocks) => Block::stack1(blocks),
            },
        ),
    }
}

/// Writes a lowercase or uppercase name onto `text`, with Elm 0.19's `_`
/// for `'`.
fn push_name(text: &mut String, name: &str) {
    for (i, part) in name.split('\'').enumerate() {
        if i > 0 {
            text.push('_');
        }
        text.push_str(part);
    }
}

/// A lowercase or uppercase name; upstream's `formatVarName'`.
fn name(name: &str) -> Line {
    let mut text = String::with_capacity(name.len());
    push_name(&mut text, name);
    Line::text(text)
}

/// `A.B.C`; upstream's `formatQualifiedUppercaseIdentifier`.
fn qualified<'a>(names: impl IntoIterator<Item = &'a Upper>) -> Line {
    let mut text = String::new();
    for (i, name) in names.into_iter().enumerate() {
        if i > 0 {
            text.push('.');
        }
        push_name(&mut text, &name.0);
    }
    Line::text(text)
}

/// `A.B.C.D`, a namespace and the name in it.
fn upper_ref(name: &UpperRef) -> Line {
    qualified(name.namespace.iter().chain([&name.name]))
}

/// An operator as a name, `(op)`.
fn operator(op: &Symbol) -> Line {
    Line::text(format!("({})", op.0))
}

/// The function making tuples of `n` elements, `(,)` and longer.
fn tuple_function(n: usize) -> Line {
    Line::text(format!("({})", ",".repeat(n - 1)))
}

fn comment(comment: &Comment) -> Block {
    match comment {
        Comment::Block(lines) => match &lines[..] {
            [] => Block::line(Line::text("{- -}")),
            [line] => Block::line(Line::row([
                Line::text("{-"),
                Line::space(),
                Line::text(line.as_str()),
                Line::space(),
                Line::text("-}"),
            ])),
            lines => Block::stack1([
                Block::stack1(lines.iter().map(|l| Block::line(Line::text(l.as_str()))))
                    .prefix(&Line::row([Line::text("{-"), Line::space()])),
                Block::line(Line::text("-}")),
            ]),
        },
        Comment::Line(text) => {
            Block::must_break(Line::row([Line::text("--"), Line::text(text.as_str())]))
        }
        Comment::TrickOpener => Block::must_break(Line::text("{--}")),
        Comment::TrickCloser => Block::must_break(Line::text("--}")),
        // Upstream drops the spaces after `{--` and before `-}`, so
        // `{-- } a -}` prints as `{--}` and code, and `{-- a { -}` as an
        // unclosed `{-}`; nomi keeps a space there (`DECISIONS.md`).
        Comment::TrickBlock(text) => Block::must_break(Line::row([
            Line::text("{--"),
            Line::text(if text.starts_with('}') { " " } else { "" }),
            Line::text(text.as_str()),
            Line::text(if text.ends_with('{') { " " } else { "" }),
            Line::text("-}"),
        ])),
    }
}

/// Upstream's `formatComments`.
fn comments<'c>(comments: impl IntoIterator<Item = &'c Comment>) -> Option<Block> {
    let mut comments = comments.into_iter();
    let first = comment(comments.next()?);
    Some(space_sep_or_stack(first, comments.map(comment).collect()))
}

/// `inner` between the comments before and after it; upstream's
/// `formatCommented`.
fn commented<'c>(
    pre: impl IntoIterator<Item = &'c Comment>,
    inner: Block,
    post: impl IntoIterator<Item = &'c Comment>,
) -> Block {
    commented_with(ForceMultiline(false), pre, inner, post)
}

/// `commented`, stacked even when it would fit on a line; upstream's
/// `formatCommented_ True`.
fn commented_forced(pre: &[Comment], inner: Block, post: &[Comment]) -> Block {
    commented_with(ForceMultiline(true), pre, inner, post)
}

fn commented_with<'c>(
    force: ForceMultiline,
    pre: impl IntoIterator<Item = &'c Comment>,
    inner: Block,
    post: impl IntoIterator<Item = &'c Comment>,
) -> Block {
    forceable_space_sep_or_stack1(
        force,
        comments(pre)
            .into_iter()
            .chain([inner])
            .chain(comments(post))
            .collect(),
    )
}

/// Upstream's `formatPreCommented`.
fn pre_commented<'c>(pre: impl IntoIterator<Item = &'c Comment>, inner: Block) -> Block {
    commented(pre, inner, [])
}

/// Upstream's `formatTailCommented`.
fn tail_commented(inner: Block, post: &[Comment]) -> Block {
    commented(&[], inner, post)
}

/// `inner`, ending in the `--` comment `eol`; upstream's
/// `formatEolCommented`.
fn eol_commented(eol: Option<&str>, inner: Block) -> Block {
    match (eol, inner) {
        (None, inner) => inner,
        (Some(eol), Block::SingleLine(line)) => Block::must_break(Line::row([
            line,
            Line::space(),
            Line::text("--"),
            Line::text(eol),
        ])),
        (Some(eol), inner) => Block::stack1([inner, comment(&Comment::Line(eol.to_owned()))]),
    }
}

/// Upstream's `formatC2Eol`.
fn c2_eol(pre: &[Comment], post: &[Comment], eol: Option<&str>, inner: Block) -> Block {
    commented(pre, eol_commented(eol, inner), post)
}

/// Upstream's `formatCommented` of a `C2`.
fn c2<T>(value: &C2<T>, render: impl FnOnce(&T) -> Block) -> Block {
    commented(
        &value.comments.before,
        render(&value.value),
        &value.comments.after,
    )
}

/// An operator and its right operand after a binary expression's left
/// one; upstream's `formatBinary` takes these as tuples.
struct BinaryOp<'a> {
    /// Whether the operator is `<|`, which puts what follows below it.
    left_pipe: bool,
    /// The comments before the operator.
    comments: &'a [Comment],
    op: Block,
    next: Block,
}

/// Operands joined by operators; upstream's `formatBinary`.
fn binary(multiline: ForceMultiline, left: Block, ops: Vec<BinaryOp>) -> Block {
    binary_from(multiline, left, &mut ops.into_iter())
}

fn binary_from(
    multiline: ForceMultiline,
    mut left: Block,
    ops: &mut std::vec::IntoIter<BinaryOp<'_>>,
) -> Block {
    while let Some(op) = ops.next() {
        if op.left_pipe {
            let head = space_sep_or_stack(
                left,
                comments(op.comments).into_iter().chain([op.op]).collect(),
            );
            let rest = nomi_base::grow_large(|| binary_from(multiline, op.next, ops));
            return forceable_space_sep_or_indented(multiline, head, vec![rest]);
        }
        let right = pre_commented(op.comments, space_sep_or_prefix(op.op, op.next));
        left = forceable_space_sep_or_indented(multiline, left, vec![right]);
    }
    left
}

/// Items after `left` and `delim`s, then `right` if there is one;
/// upstream's `formatSequence`.
fn sequence<T>(
    left: char,
    delim: char,
    right: Option<char>,
    multiline: ForceMultiline,
    trailing: &[Comment],
    items: &Sequence<T>,
    render: impl Fn(&T) -> Block,
) -> Block {
    let item = |delim: char, around: &SeparatorComments, value: &T| {
        let item = c2_eol(
            &around.after_separator,
            &[],
            around.eol.as_deref(),
            render(value),
        )
        .prefix(&Line::row([Line::text(delim), Line::space()]));
        match comments(&around.before_separator) {
            Some(before) => Block::blank_line().stack(before).stack(item),
            None => item,
        }
    };
    let Some((first, rest)) = items.0.split_first() else {
        return unit(left, right.unwrap_or(' '), trailing);
    };
    let first = item(left, &first.comments, &first.value);
    let rest = rest
        .iter()
        .map(|i| item(delim, &i.comments, &i.value))
        .collect();
    let after = comments(trailing)
        .map(|c| Block::blank_line().stack(c))
        .into_iter()
        .chain(right.map(|r| Block::line(Line::text(r))))
        .collect();
    forceable_space_sep_or_stack(
        multiline,
        forceable_row_or_stack(multiline, first, rest),
        after,
    )
}

/// A record, or with a base an extension; upstream's `formatRecordLike`.
fn record_like<T>(
    base: Option<&C2<Lower>>,
    fields: &Sequence<T>,
    trailing: &[Comment],
    multiline: ForceMultiline,
    render: impl Fn(&T) -> Block,
) -> Block {
    match base {
        Some(base) => extension_group_fields(
            multiline,
            c2(base, |b| Block::line(name(&b.0))),
            sequence('|', ',', None, multiline, trailing, fields, render),
        ),
        None => sequence('{', ',', Some('}'), multiline, trailing, fields, render),
    }
}

/// `key delim value`; upstream's `formatPair`.
fn pair<V>(delim: &str, pair: &Pair<V>, value: impl FnOnce(&V) -> Block) -> Block {
    structure::equals_pair(
        delim,
        pair.force_multiline,
        tail_commented(Block::line(name(&pair.key.value.0)), &pair.key.comments),
        pre_commented(&pair.value.comments, value(&pair.value.value)),
    )
}

/// `word value`, with comments around `word`; upstream's
/// `formatKeywordCommented`.
fn keyword_commented(word: &str, pre: &[Comment], post: &[Comment], value: Block) -> Block {
    space_sep_or_indented(
        commented(pre, Block::line(Line::text(word)), post),
        vec![value],
    )
}

/// A doc comment: its Markdown, with the Elm in code blocks formatted
/// and the `@docs` names cleaned; upstream's `formatDocComment`.
///
/// Reprinting the Markdown can undo the nesting of a comment inside, so
/// that upstream's comment ends early or swallows the code after it;
/// nomi then prints the comment as written (`DECISIONS.md`).
fn doc_comment(docs: &Docs) -> Block {
    let block = doc_comment_string(&markdown::render(&docs.blocks, &|code: &Option<
        ElmCode,
    >| {
        code.as_ref().map(elm_code)
    }));
    if one_doc_comment(&block.render()) {
        return block;
    }
    Block::stack1(docs.source.lines().map(|l| Block::line(Line::text(l))))
}

/// Whether `text` lexes as one doc comment and nothing else.
fn one_doc_comment(text: &str) -> bool {
    let lexed = nomi_syntax::lex(text);
    let mut tokens = lexed
        .tokens()
        .iter()
        .filter(|t| !matches!(t.kind, SyntaxKind::Whitespace | SyntaxKind::Newline));
    lexed.errors().is_empty()
        && tokens
            .next()
            .is_some_and(|t| t.kind == SyntaxKind::DocComment)
        && tokens.next().is_none()
}

/// The Elm of a code block, formatted with one blank line between
/// top-level parts.
fn elm_code(code: &ElmCode) -> String {
    let block = match code {
        ElmCode::Module(module) => format_module(module, ModuleContext::CodeBlock),
        ElmCode::Declarations(decls) => {
            declarations::body(ModuleContext::CodeBlock.spacing(), decls)
        }
        ElmCode::Expressions(exprs) => {
            declarations::expressions(ModuleContext::CodeBlock.spacing(), exprs)
        }
    };
    block.map_or_else(String::new, |b| b.render())
}

/// A doc comment of `docs`, already rendered; upstream's
/// `formatDocCommentString`.
fn doc_comment_string(docs: &str) -> Block {
    // Haskell's `lines`: no last line after a final newline.
    let mut lines = docs.split_terminator('\n');
    let Some(first) = lines.next() else {
        return Block::line(Line::row([
            Line::text("{-|"),
            Line::space(),
            Line::text("-}"),
        ]));
    };
    Block::line(Line::row([
        Line::text("{-|"),
        Line::space(),
        Line::text(first),
    ]))
    .and_then(lines.map(|l| Block::line(Line::text(l))))
    .and_then([Block::line(Line::text("-}"))])
}

/// What upstream prints where it meets what it believes can't happen.
fn please_report(what: &str, details: &str) -> Block {
    text(format!(
        "<elm-format: {what}: {details} -- please report this at https://github.com/avh4/elm-format/issues >"
    ))
}

#[cfg(test)]
mod tests;
