//! The Markdown round trip against elm-format 0.8.8: each case is a doc
//! comment's text after `{-|`, and what upstream printed there. Code
//! blocks stay as written; the fixtures cover formatted ones. Lines lose
//! their trailing spaces, as the doc comment's box does.

use super::{parse, render};

#[test]
fn doc_comments_print_as_upstream_prints_them() {
    let cases = [
        (
            " Text with *emph*, __strong__ and `code`.\n",
            "Text with _emph_, **strong** and `code`.\n",
        ),
        (
            " # Title\n\nText\n\nSub\n---\n",
            "\n\n\n# Title\n\nText\n\n\n## Sub\n\n",
        ),
        (
            " A\n\n* a\n* b\n\n1) x\n2) y\n",
            "A\n\n  - a\n  - b\n\n1.  x\n2.  y\n\n",
        ),
        (" A\n\n- a\n\n- b\n", "A\n\n  - a\n\n  - b\n\n"),
        (" A\n\n10. a\n11. b\n", "A\n\n1.  a\n2.  b\n\n"),
        (" > quoted\n> text\n", "\n\n> quoted\n> text\n\n"),
        (
            " <https://x.y> and [t](http://a \"T\") and ![i](p.png)\n",
            "<https://x.y> and [t](http://a \"T\") and ![i](p.png)\n",
        ),
        (
            " A [ref] and [x][ref].\n\n[ref]: http://r \"T\"\n",
            "A [ref] and [x][ref].\n\n[ref]: http://r \"T\"\n\n",
        ),
        (" a_b \\* c \\\\ d\n", "a\\_b \\* c \\\\ d\n"),
        (" A\n\n```js\nx\n\n```\n", "A\n\n```js\nx\n```\n\n"),
        (
            " A\n\n    {- raw -} x = 1\n\n\n",
            "A\n\n    {- raw -} x = 1\n\n",
        ),
        (" @docs a, b(..)\n@docs c\n", "\n\n@docs a, b\n@docs c\n\n"),
        (" A\n\n@docs a, b(..)\n", "A\n\n@docs a, b\n\n"),
        (" A\n\n***\n", "A\n\n---\n\n"),
        (" A\n\n<div>\nx\n</div>\n", "A\n\n<div>\nx\n</div>\n\n"),
        (
            " `` a`b `` and a  \nb &amp; c\n",
            "`` a`b `` and a\nb &amp; c\n",
        ),
        (
            " - a\n    - b\n\n  c\n",
            "\n\n  - a\n\n      - b\n\n    c\n\n",
        ),
        (
            " A\n\n- a\n\n      {- raw -} x = 1\n- b\n",
            "A\n\n  - a\n\n        {- raw -} x = 1\n\n  - b\n\n",
        ),
    ];
    for (text, expected) in cases {
        let blocks = parse(text, &mut |_, _| ());
        let printed: String = render(&blocks, &|()| None)
            .split_inclusive('\n')
            .map(|line| format!("{}\n", line.trim_end()))
            .collect();
        assert_eq!(printed, expected, "{text:?}");
    }
}
