//! Upstream's `ElmStructureTest`, and each layout's other cases.

use super::*;
use crate::tree::{ApplicationMultiline::*, Multiline::*};

fn word(text: &str) -> Block {
    Block::line(Line::text(text))
}

/// Two lines, each `text` twice.
fn block(text: &str) -> Block {
    let twice = || Block::line(Line::row([Line::text(text), Line::text(text)]));
    Block::stack1([twice(), twice()])
}

fn comment(text: &str) -> Block {
    Block::must_break(Line::text(format!("-- {text}")))
}

fn words(texts: &[&str]) -> Vec<Block> {
    texts.iter().map(|t| word(t)).collect()
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
    assert_eq!(group(true, "(", ",", ")", false, vec![]).render(), "()\n");
    assert_eq!(group(true, "(", ",", ")", true, vec![]).render(), "()\n");
    assert_eq!(
        group(true, "(", ",", ")", false, words(&["foo"])).render(),
        "( foo )\n"
    );
    assert_eq!(
        group(true, "(", ",", ")", false, words(&["foo", "bar"])).render(),
        "( foo, bar )\n"
    );
    assert_eq!(
        group(false, "(", ",", ")", false, words(&["foo", "bar"])).render(),
        "(foo, bar)\n"
    );
    assert_eq!(
        group(
            true,
            "(",
            ",",
            ")",
            false,
            vec![block("a"), word("b"), block("c")]
        )
        .render(),
        "( aa\n  aa\n, b\n, cc\n  cc\n)\n"
    );
    assert_eq!(
        group(true, "(", ",", ")", true, words(&["a", "b", "c"])).render(),
        "( a\n, b\n, c\n)\n"
    );
}

#[test]
fn groups_with_footers() {
    let footer = || vec![word("..")];
    assert_eq!(
        group_with_footer(true, "(", ",", footer(), ")", false, vec![]).render(),
        "(..)\n"
    );
    assert_eq!(
        group_with_footer(true, "(", ",", footer(), ")", false, words(&["a"])).render(),
        "( a, .. )\n"
    );
    assert_eq!(
        group_with_footer(
            true,
            "(",
            ",",
            vec![comment("c")],
            ")",
            false,
            words(&["a"])
        )
        .render(),
        "( a\n-- c\n)\n"
    );
    // Upstream drops a multiline footer when there are no children.
    assert_eq!(
        group_with_footer(true, "(", ",", vec![comment("c")], ")", false, vec![]).render(),
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
        forceable_space_sep_or_stack(true, word("a"), words(&["b"])).render(),
        "a\nb\n"
    );
    assert_eq!(
        space_sep_or_stack(comment("a"), words(&["b"])).render(),
        "-- a\nb\n"
    );
    assert_eq!(
        forceable_row_or_stack(false, word("a"), words(&["b"])).render(),
        "ab\n"
    );
    assert_eq!(
        forceable_space_sep_or_stack1(false, words(&["a", "b"])).render(),
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
        equals_pair("=", false, word("a"), word("b")).render(),
        "a = b\n"
    );
    assert_eq!(
        equals_pair("=", true, word("a"), word("b")).render(),
        "a =\n    b\n"
    );
    assert_eq!(
        equals_pair("=", true, word("a"), comment("b")).render(),
        "a = -- b\n"
    );
    assert_eq!(
        equals_pair(":", false, block("a"), word("b")).render(),
        "aa\naa\n    :\n    b\n"
    );
    assert_eq!(
        definition("=", false, word("f"), words(&["x", "y"]), word("x")).render(),
        "f x y = x\n"
    );
}

#[test]
fn extension_groups() {
    assert_eq!(
        extension_group(false, word("r"), word("a"), words(&["b"])).render(),
        "{ r | a, b }\n"
    );
    assert_eq!(
        extension_group(true, word("r"), word("a"), words(&["b"])).render(),
        "{ r\n    | a\n    , b\n}\n"
    );
    assert_eq!(
        extension_group_fields(false, word("r"), word("| a")).render(),
        "{ r | a }\n"
    );
    assert_eq!(
        extension_group_fields(false, word("r"), block("a")).render(),
        "{ r\n    aa\n    aa\n}\n"
    );
}
