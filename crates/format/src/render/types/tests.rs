//! Expected outputs are elm-format 0.8.8's, for `x : <type>`.

use crate::lower::type_fragment;

use super::super::structure::{DefinitionKind, definition};
use super::super::text;
use super::type_alone;

/// `x : source`, as upstream's `formatTypeAnnotation` lays it out.
fn annotation(source: &str) -> String {
    let ty = type_fragment(source);
    let block = definition(
        DefinitionKind::Annotation,
        text("x"),
        Vec::new(),
        type_alone(&ty),
    );
    block.render()
}

fn assert_annotates(text: &str, expected: &str) {
    assert_eq!(annotation(text), expected, "{text}");
}

#[test]
fn units_and_parentheses() {
    assert_annotates(
        "( ) -> ({- c -}) -> {} -> { -- x\n  }",
        "x :\n    ()\n    -> ({- c -})\n    -> {}\n    ->\n        { -- x\n        }\n",
    );
    assert_annotates(
        "((Int)) -> (List (Maybe a) -> b) -> (a -> b) -> c",
        "x : Int -> (List (Maybe a) -> b) -> (a -> b) -> c\n",
    );
}

#[test]
fn records_and_extensions() {
    assert_annotates(
        "{ r | x : Int {- k -}, y :\n   Int }",
        "x :\n    { r\n        | x : Int\n\n        {- k -}\n        , y :\n            Int\n    }\n",
    );
    assert_annotates(
        "{ a : Int, -- e\n    {- pre -} b : Int }",
        "x :\n    { a : Int\n    , -- e\n      {- pre -}\n      b : Int\n    }\n",
    );
    assert_annotates("{} -> { r | x : Int }", "x : {} -> { r | x : Int }\n");
    assert_annotates(
        "{ first: Int\n\n    -- comment\n    , second: Int\n    -- trailing\n    }",
        "x :\n    { first : Int\n\n    -- comment\n    , second : Int\n\n    -- trailing\n    }\n",
    );
}

#[test]
fn tuples_and_tuple_constructors() {
    assert_annotates(
        "(Int, -- e\n   String) -> (,,) a b c",
        "x :\n    ( Int\n    , -- e\n      String\n    )\n    -> (,,) a b c\n",
    );
}

#[test]
fn functions_keep_comments_by_their_arrows() {
    assert_annotates(
        "Int\n  -> {- before -} String -- after\n  -> Int",
        "x :\n    Int\n    ->\n        {- before -}\n        String -- after\n    -> Int\n",
    );
    assert_annotates("Int -- one\n  -> Int", "x :\n    Int -- one\n    -> Int\n");
}

#[test]
fn constructions_broken_in_the_source_stay_broken() {
    assert_annotates(
        "Dict\n   String Int -> Int",
        "x :\n    Dict\n        String\n        Int\n    -> Int\n",
    );
    assert_annotates("a -> List {- a -} Int", "x : a -> List {- a -} Int\n");
}
