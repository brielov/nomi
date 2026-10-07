use super::*;
use crate::tree::Upper;

fn render(block: Block) -> String {
    block.render()
}

#[test]
fn comments_render_as_upstream_spells_them() {
    let cases = [
        (Comment::Block(vec![]), "{- -}\n"),
        (Comment::Block(vec!["a".to_owned()]), "{- a -}\n"),
        (
            Comment::Block(vec!["a".to_owned(), "  b".to_owned()]),
            "{- a\n     b\n-}\n",
        ),
        (Comment::Line(" a".to_owned()), "-- a\n"),
        (Comment::TrickOpener, "{--}\n"),
        (Comment::TrickCloser, "--}\n"),
        (Comment::TrickBlock("a".to_owned()), "{--a-}\n"),
    ];
    for (c, expected) in cases {
        assert_eq!(render(comment(&c)), expected, "{c:?}");
    }
}

#[test]
fn line_comments_break_what_they_join() {
    let inner = Block::line(Line::text("x"));
    let pre = [Comment::Block(vec!["a".to_owned()])];
    assert_eq!(render(commented(&pre, inner.clone(), &[])), "{- a -} x\n");
    let post = [Comment::Line(" b".to_owned())];
    assert_eq!(render(commented(&pre, inner, &post)), "{- a -}\nx\n-- b\n");
}

#[test]
fn doc_comments_close_on_their_own_line() {
    assert_eq!(render(doc_comment_string("")), "{-| -}\n");
    assert_eq!(render(doc_comment_string("A.\n")), "{-| A.\n-}\n");
    assert_eq!(
        render(doc_comment_string("A.\n\nB.\n")),
        "{-| A.\n\nB.\n-}\n"
    );
}

#[test]
fn names_spell_primes_as_underscores() {
    assert_eq!(render(Block::line(name("a'b"))), "a_b\n");
    let names = [Upper("A'".to_owned()), Upper("B".to_owned())];
    assert_eq!(render(Block::line(qualified(&names))), "A_.B\n");
}

#[test]
fn the_body_follows_the_header_after_blank_lines() {
    let format = crate::format;
    // Three before a comment, two before a declaration, and none
    // without a body.
    assert_eq!(
        format("module A exposing (a)\nimport B\n-- c\n\n\na =\n    1\n").as_deref(),
        Some("module A exposing (a)\n\nimport B\n\n\n\n-- c\n\n\na =\n    1\n")
    );
    assert_eq!(
        format("module A exposing (a)\na =\n    1\n").as_deref(),
        Some("module A exposing (a)\n\n\na =\n    1\n")
    );
    assert_eq!(
        format("module A exposing (a)\nimport B\n").as_deref(),
        Some("module A exposing (a)\n\nimport B\n")
    );
}
