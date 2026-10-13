//! Expected outputs are elm-format 0.8.8's. The declarations in each
//! input are already formatted, so each case checks the header alone.

use crate::test_support::assert_formats;

#[test]
fn a_module_without_a_header_exposes_what_it_defines() {
    let body = "type alias R =\n    {}\n\n\ntype T\n    = A\n\n\nx =\n    1\n\n\n{ r, s } =\n    { r = 1, s = 2 }\n\n\n(+++) =\n    1\n";
    assert_formats(
        body,
        &format!("module Main exposing (R, T(..), r, s, x)\n\n\n{body}"),
    );
}

/// Upstream also drops the blank lines between the two, as if the second
/// were a definition of the first.
#[test]
fn a_type_and_an_alias_of_one_name_both_stay_exposed() {
    assert_formats(
        "type alias Y =\n    ()\n\n\ntype Y\n    = Y0\n",
        "module Main exposing (Y(..), Y)\n\n\ntype alias Y =\n    ()\ntype Y\n    = Y0\n",
    );
}

#[test]
fn docs_group_the_exposing_list() {
    let docs = "{-| Docs.\n\n@docs a, b, nope\n@docs T, (+)\n\n    @docs D\n\n-}\n\nimport B\n\n\na =\n    1\n";
    assert_formats(
        &format!("module A exposing (b, a, T, (+), C(..), D(E))\n\n{docs}"),
        &format!(
            "module A exposing\n    ( a, b\n    , T, (+)\n    , C(..), D(..)\n    )\n\n{docs}"
        ),
    );
}

#[test]
fn a_broken_list_breaks_after_each_item() {
    assert_formats(
        "module A exposing\n    ( b {- x -}\n    , a\n    , c -- y\n    )\n\n\na =\n    1\n",
        "module A exposing\n    ( a\n    , b {- x -}\n    , c\n      -- y\n    )\n\n\na =\n    1\n",
    );
}

#[test]
fn items_the_docs_move_lose_their_comments() {
    assert_formats(
        "module A exposing (b {- x -}, a)\n\n{-|\n\n@docs a\n\n-}\n\nimport B\n\n\na =\n    1\n",
        "module A exposing\n    ( a\n    , b {- x -}\n    )\n\n{-|\n\n@docs a\n\n-}\n\nimport B\n\n\na =\n    1\n",
    );
    assert_formats(
        "module A exposing (a {- x -}, b)\n\n{-|\n\n@docs a\n\n-}\n\nimport B\n\n\na =\n    1\n",
        "module A exposing\n    ( a\n    , b\n    {- x -}\n    )\n\n{-|\n\n@docs a\n\n-}\n\nimport B\n\n\na =\n    1\n",
    );
}

#[test]
fn imports_sort_merge_and_lose_redundant_aliases() {
    assert_formats(
        "-- first\nport {- p -} module {- n -} A exposing (..)\n\nimport List as List\nimport Dict as D exposing (Dict, get)\n-- c\nimport A.B as B\nimport C exposing (\n  x\n  , y)\nimport E {- e -} as {- f -} F\n\n\n\n-- body\n\n\na =\n    1\n",
        "-- first\n\n\nport {- p -} module {- n -} A exposing (..)\n\n-- c\n\nimport A.B as B\nimport C\n    exposing\n        ( x\n        , y\n        )\nimport Dict as D exposing (Dict, get)\nimport E {- e -} as {- f -} F\nimport List\n\n\n\n-- body\n\n\na =\n    1\n",
    );
}

#[test]
fn effect_settings() {
    let text = "effect module A where { subscription = MySub, command = {- c -} MyCmd } exposing (a)\n\n\na =\n    1\n";
    assert_formats(text, text);
}

#[test]
fn listings_put_operators_then_types_then_values() {
    assert_formats(
        "module A exposing (a)\n\nimport B exposing (T {- t -} (..), (+), x)\nimport C exposing ( .. )\n",
        "module A exposing (a)\n\nimport B exposing ((+), T(..) {- t -}, x)\nimport C exposing (..)\n",
    );
}

#[test]
fn doc_terms_name_operators_of_up_to_two_characters() {
    assert_eq!(super::ref_name("(+)"), "+");
    assert_eq!(super::ref_name("(<|)"), "<|");
    assert_eq!(super::ref_name("(<=>)"), "(<=>)");
    assert_eq!(super::ref_name("Maybe"), "Maybe");
}
