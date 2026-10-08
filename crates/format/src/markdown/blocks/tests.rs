//! The block parser, on what lowering relies on: the `@docs` groups that
//! order the exposing list, and the code blocks it hands to the Elm
//! parser, each probed against elm-format 0.8.8.

use super::super::{Block, CodeAttr};
use super::parse;

/// Each code block's language and text.
fn code_blocks(text: &str) -> Vec<(String, String)> {
    parse(text, &mut |attr: &CodeAttr, code: &str| {
        (attr.lang.clone(), code.to_owned())
    })
    .into_iter()
    .filter_map(|block| match block {
        Block::Code { code, .. } => Some(code),
        _ => None,
    })
    .collect()
}

#[test]
fn docs_lines_group_their_names() {
    let blocks = parse("A\n\n@docs a, b(..)\n@docs c\n", &mut |_, _| ());
    assert_eq!(
        blocks[1],
        Block::ElmDocs(vec![
            vec!["a".to_owned(), "b(..)".to_owned()],
            vec!["c".to_owned()],
        ])
    );
}

#[test]
fn indented_code_loses_its_last_blank_lines() {
    assert_eq!(
        code_blocks("A\n\n    x\n\n    y\n\n\n"),
        [(String::new(), "x\n\ny".to_owned())]
    );
}

#[test]
fn fenced_code_keeps_its_language_and_blank_lines() {
    assert_eq!(
        code_blocks("```elm\nx\n\n```\n\n```js extra\n```"),
        [
            ("elm".to_owned(), "x\n".to_owned()),
            ("js".to_owned(), String::new()),
        ]
    );
}

#[test]
fn code_in_a_list_item_is_the_item_s() {
    let blocks = parse(
        "- a\n\n      x\n- b\n",
        &mut |attr: &CodeAttr, code: &str| (attr.lang.clone(), code.to_owned()),
    );
    let [Block::List { tight, items, .. }] = &blocks[..] else {
        panic!("{blocks:?}");
    };
    assert!(!tight);
    assert!(matches!(&items[0][1], Block::Code { text, .. } if text == "x"));
}
