//! Upstream's `ElmStructureTest`, and each layout's other cases.

use super::super::text as word;
use super::*;
use crate::tree::{ApplicationMultiline::*, Multiline::*};

const FORCED: ForceMultiline = ForceMultiline(true);
const FREE: ForceMultiline = ForceMultiline(false);

/// Two lines, each `text` twice.
fn block(text: &str) -> Block {
    let twice = || Block::line(Line::row([Line::text(text), Line::text(text)]));
    Block::stack1([twice(), twice()])
}

fn comment(text: &str) -> Block {
    Block::must_break(Line::text(format!("-- {text}")))
}

fn words(texts: &[&str]) -> Vec<Block> {
    texts.iter().map(|&t| word(t)).collect()
}

#[test]
fn applications() {
    let join = JoinFirst(JoinAll);
    assert_eq!(
        application(join, word("a"), words(&["b", "c"])).render(),
        "a b c\n"
    );
    assert_eq!(
        application(join, block("a"), vec![block("b"), word("c")]).render(),
        "aa\naa\n    bb\n    bb\n    c\n"
    );
    assert_eq!(application(join, word("f"), vec![]).render(), "f\n");
    assert_eq!(
        application(JoinFirst(SplitAll), word("f"), words(&["a", "b"])).render(),
        "f a\n    b\n"
    );
    assert_eq!(
        application(join, word("f"), vec![word("a"), block("b")]).render(),
        "f a\n    bb\n    bb\n"
    );
    assert_eq!(
        application(SplitFirst, word("f"), words(&["a", "b"])).render(),
        "f\n    a\n    b\n"
    );
}

#[test]
fn groups() {
    let padded = Delimiters::PADDED_PARENS;
    assert_eq!(group(padded, FREE, vec![]).render(), "()\n");
    assert_eq!(group(padded, FORCED, vec![]).render(), "()\n");
    assert_eq!(group(padded, FREE, words(&["foo"])).render(), "( foo )\n");
    assert_eq!(
        group(padded, FREE, words(&["foo", "bar"])).render(),
        "( foo, bar )\n"
    );
    assert_eq!(
        group(Delimiters::PARENS, FREE, words(&["foo", "bar"])).render(),
        "(foo, bar)\n"
    );
    assert_eq!(
        group(Delimiters::NONE, FREE, words(&["foo", "bar"])).render(),
        "foo, bar\n"
    );
    assert_eq!(
        group(padded, FREE, vec![block("a"), word("b"), block("c")]).render(),
        "( aa\n  aa\n, b\n, cc\n  cc\n)\n"
    );
    assert_eq!(
        group(padded, FORCED, words(&["a", "b", "c"])).render(),
        "( a\n, b\n, c\n)\n"
    );
}

#[test]
fn groups_with_footers() {
    let padded = Delimiters::PADDED_PARENS;
    let footer = || vec![word("{-c-}")];
    assert_eq!(
        group_with_footer(padded, footer(), FREE, vec![]).render(),
        "({-c-})\n"
    );
    assert_eq!(
        group_with_footer(padded, footer(), FREE, words(&["a", "b"])).render(),
        "( a, b {-c-} )\n"
    );
    assert_eq!(
        group_with_footer(padded, vec![comment("c")], FREE, words(&["a"])).render(),
        "( a\n-- c\n)\n"
    );
    // Upstream drops a multiline footer when there are no children.
    assert_eq!(
        group_with_footer(padded, vec![comment("c")], FREE, vec![]).render(),
        "()\n"
    );
}

#[test]
fn space_separated() {
    assert_eq!(
        space_sep_or_stack(word("a"), words(&["b", "c"])).render(),
        "a b c\n"
    );
    assert_eq!(
        forceable_space_sep_or_stack(FORCED, word("a"), words(&["b"])).render(),
        "a\nb\n"
    );
    assert_eq!(
        space_sep_or_stack(comment("a"), words(&["b"])).render(),
        "-- a\nb\n"
    );
    assert_eq!(
        forceable_row_or_stack(FREE, word("a"), words(&["b"])).render(),
        "ab\n"
    );
    assert_eq!(
        forceable_space_sep_or_stack1(FREE, words(&["a", "b"])).render(),
        "a b\n"
    );
    assert_eq!(
        space_sep_or_indented(word("a"), words(&["b"])).render(),
        "a b\n"
    );
    assert_eq!(
        space_sep_or_indented(word("a"), vec![word("b"), comment("c")]).render(),
        "a\n    b\n    -- c\n"
    );
}

#[test]
fn prefixed() {
    assert_eq!(space_sep_or_prefix(word("-"), word("a")).render(), "- a\n");
    assert_eq!(
        space_sep_or_prefix(word("<|"), block("a")).render(),
        "<| aa\n   aa\n"
    );
    assert_eq!(
        space_sep_or_prefix(word("opLong"), block("a")).render(),
        "opLong\n    aa\n    aa\n"
    );
    // Three columns and the space make one tab's width.
    assert_eq!(
        space_sep_or_prefix(word("|>>"), block("a")).render(),
        "|>> aa\n    aa\n"
    );
    assert_eq!(prefix_or_indented(word("a"), word("b")).render(), "a b\n");
    let must_break = prefix_or_indented(word("a"), comment("b"));
    assert_eq!(
        (must_break.is_line(), must_break.render()),
        (false, "a -- b\n".into())
    );
    assert_eq!(
        prefix_or_indented(word("a"), block("b")).render(),
        "a\n    bb\n    bb\n"
    );
}

#[test]
fn equals_pairs() {
    assert_eq!(
        equals_pair("=", FREE, word("a"), word("b")).render(),
        "a = b\n"
    );
    assert_eq!(
        equals_pair("=", FORCED, word("a"), word("b")).render(),
        "a =\n    b\n"
    );
    assert_eq!(
        equals_pair("=", FORCED, word("a"), comment("b")).render(),
        "a = -- b\n"
    );
    assert_eq!(
        equals_pair(":", FREE, block("a"), word("b")).render(),
        "aa\naa\n    :\n    b\n"
    );
    assert_eq!(
        definition(
            DefinitionKind::Annotation,
            word("f"),
            words(&["x", "y"]),
            word("x")
        )
        .render(),
        "f x y : x\n"
    );
    assert_eq!(
        definition(DefinitionKind::Value, word("f"), words(&["x"]), word("x")).render(),
        "f x =\n    x\n"
    );
}

#[test]
fn extension_groups() {
    assert_eq!(
        extension_group_fields(FREE, word("r"), word("| a")).render(),
        "{ r | a }\n"
    );
    assert_eq!(
        extension_group_fields(FREE, word("r"), block("a")).render(),
        "{ r\n    aa\n    aa\n}\n"
    );
}
