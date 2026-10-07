//! Tree snapshots for every file under `test_data/parser`. Run with
//! `INSTA_UPDATE=always` after a grammar change and review the diffs.

use std::fmt::Write;

fn render(text: &str) -> String {
    let parse = nomi_syntax::parse_module(text);
    assert_eq!(parse.syntax().to_string(), text, "the tree is lossless");
    let mut out = parse.debug_tree();
    for error in parse.errors() {
        writeln!(out, "error {:?}: {:?}", error.range, error.kind).unwrap();
    }
    out
}

#[test]
fn module() {
    insta::glob!("../test_data/parser", "module/*.elm", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        insta::assert_snapshot!(render(&text));
    });
}
