//! Upstream's `BoxTest`, and the edges of `Box.hs` the formatter leans on.

use super::*;

fn word(text: &str) -> Block {
    Block::line(Line::text(text))
}

#[test]
fn lines_render_their_pieces() {
    assert_eq!(word("module").render(), "module\n");
    assert_eq!(
        Block::line(Line::row([Line::text("a"), Line::text("b")])).render(),
        "ab\n"
    );
    assert_eq!(
        Block::line(Line::row([Line::text("a"), Line::space(), Line::text("b")])).render(),
        "a b\n"
    );
}

#[test]
fn stacks_and_indents() {
    let ab = || Block::stack1([word("a"), word("b")]);
    assert_eq!(
        Block::stack1([word("foo"), word("bar")]).render(),
        "foo\nbar\n"
    );
    assert_eq!(ab().indent().render(), "    a\n    b\n");
    assert_eq!(ab().indent().indent().render(), "        a\n        b\n");
    // A tab advances to the next multiple of four.
    assert_eq!(
        word("a").indent().prefix(&Line::space()).render(),
        "    a\n"
    );
    assert_eq!(
        ab().indent().prefix(&Line::text("[ ")).render(),
        "[   a\n    b\n"
    );
    assert_eq!(
        word("a").indent().prefix(&Line::text("12345")).render(),
        "12345   a\n"
    );
}

#[test]
fn prefixes_pad_the_lines_after_the_first() {
    let block = Block::stack1([word("abcde"), word("xyz")]);
    assert_eq!(
        block.prefix(&Line::text("myPrefix ")).render(),
        "myPrefix abcde\n         xyz\n"
    );
    // Padding counts characters, not bytes.
    let block = Block::stack1([word("a"), word("b")]);
    assert_eq!(block.prefix(&Line::text("→ ")).render(), "→ a\n  b\n");
}

#[test]
fn single_lines_and_must_breaks() {
    let comment = Block::must_break(Line::text("-- note"));
    assert!(!comment.is_line());
    assert!(comment.clone().indent().into_line().is_err());
    // Upstream's `addSuffix` makes any single line joinable again.
    assert!(comment.add_suffix(Line::text("!")).is_line());
    let stack = Block::stack1([word("a"), word("b")]).add_suffix(Line::text(","));
    assert_eq!(stack.render(), "a\nb,\n");
    // Stacking a single line makes a stack, whatever its kind.
    let stacked = Block::must_break(Line::text("-- a")).stack(word("b"));
    assert!(matches!(stacked, Block::Stack(ref lines) if lines.len() == 2));
}

#[test]
fn and_then_stacks_below() {
    let block = word("a").and_then([word("b"), Block::stack1([word("c"), word("d")])]);
    assert_eq!(block.render(), "a\nb\nc\nd\n");
    assert_eq!(word("a").and_then([]), word("a"));
}

#[test]
fn all_singles_needs_every_block_joinable() {
    assert_eq!(
        Block::all_singles(vec![word("a"), word("b")]),
        Ok(vec![Line::text("a"), Line::text("b")])
    );
    let mixed = vec![word("a"), Block::must_break(Line::text("-- b"))];
    assert_eq!(Block::all_singles(mixed.clone()), Err(mixed));
}

#[test]
fn rendering_strips_what_haskell_calls_space() {
    assert_eq!(word("a \t\u{a0}\u{3000}").render(), "a\n");
    // `isSpace` is false for these, unlike `char::is_whitespace`.
    for kept in ["a\u{85}", "a\u{2028}", "a\u{2029}"] {
        assert_eq!(word(kept).render(), format!("{kept}\n"));
    }
    // An indented blank line is empty.
    assert_eq!(Block::blank_line().indent().render(), "\n");
}

#[test]
fn lengths_follow_tabs_from_the_start_column() {
    let line = Line::row([Line::text("ab"), Line::space()]);
    assert_eq!(line.length(0), 3);
    assert_eq!(line.length(5), 8);
    let indented = word("x").indent().into_line().unwrap();
    assert_eq!(indented.length(0), 5);
    assert_eq!(indented.length(2), 5);
}

#[test]
fn deep_indentation_is_linear() {
    // Each level prepends one tab to each line; nothing is copied.
    let mut block = word("x");
    for _ in 0..20_000 {
        block = block.indent();
    }
    let out = block.render();
    assert_eq!(out.len(), 80_000 + 2);
}
