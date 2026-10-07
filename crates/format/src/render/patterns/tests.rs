//! Expected outputs are elm-format 0.8.8's, from case branches.

use crate::lower::pattern_fragment;

use super::pattern_alone;

fn assert_renders(text: &str, expected: &str) {
    let rendered = pattern_alone(&pattern_fragment(text)).render();
    assert_eq!(rendered, expected, "{text}");
}

#[test]
fn single_tokens_and_units() {
    assert_renders("_", "_\n");
    assert_renders("()", "()\n");
    assert_renders("( {- a -} )", "({- a -})\n");
    assert_renders("{}", "{}\n");
    assert_renders("{ -- c\n  }", "{ -- c\n}\n");
    assert_renders("True", "True\n");
    assert_renders("0x1F", "0x1F\n");
    assert_renders("1e22", "1.0e22\n");
}

#[test]
fn arguments_take_parentheses_where_they_need_them() {
    assert_renders(
        "Just (Ok y) :: (a,b) :: rest",
        "(Just (Ok y)) :: ( a, b ) :: rest\n",
    );
    assert_renders("(Just a as b)", "(Just a) as b\n");
    assert_renders("Foo.Bar a {- c -} _", "Foo.Bar a {- c -} _\n");
    assert_renders("((x))", "x\n");
}

#[test]
fn delimited_patterns() {
    assert_renders("{ a, b {- c -} }", "{ a, b {- c -} }\n");
    assert_renders("[ a, [] , [ {- c -} ] ]", "[ a, [], [{- c -}] ]\n");
}

#[test]
fn line_comments_break_operators_and_aliases() {
    assert_renders("a :: -- e\n     b", "a\n    :: -- e\n       b\n");
    assert_renders("(a as -- c\n      b)", "a\nas\n    -- c\n    b\n");
}
