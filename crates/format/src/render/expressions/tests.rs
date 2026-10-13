//! Expected outputs are elm-format 0.8.8's, for a module's declarations.

use crate::test_support::lowered;
use crate::tree::{Declaration, TopLevelStructure};

use super::super::declarations::common;

/// The definitions and annotations in `decls`, rendered one after the
/// other.
fn render(decls: &str) -> String {
    lowered(&format!("module M exposing (..)\n\n\n{decls}"))
        .body
        .iter()
        .filter_map(|item| match item {
            TopLevelStructure::Entry(Declaration::Common(decl)) => Some(common(decl).render()),
            _ => None,
        })
        .collect()
}

#[track_caller]
fn assert_renders(decls: &str, expected: &str) {
    assert_eq!(render(decls), expected, "{decls}");
}

#[test]
fn operators_group_and_left_pipes_end_lambdas() {
    assert_renders(
        "ops = a + b * (c - d) |> f <| \\x -> x\n",
        "ops =\n    a + b * (c - d) |> f <| \\x -> x\n",
    );
    assert_renders("pipes = f <| g <| h x\n", "pipes =\n    f <| g <| h x\n");
    assert_renders(
        "neg = -x + -(f y) - (-3)\n",
        "neg =\n    -x + -(f y) - -3\n",
    );
    assert_renders(
        "parens = ({- a -} x {- b -}) + ((y))\n",
        "parens =\n    ({- a -} x {- b -}) + y\n",
    );
}

#[test]
fn applications_break_after_the_first_argument() {
    assert_renders(
        "app = f (g x) (h\n  y) z\n",
        "app =\n    f (g x)\n        (h\n            y\n        )\n        z\n",
    );
}

#[test]
fn lambdas() {
    assert_renders(
        "lambda = \\(Just a) {b} _ ->\n  a\n",
        "lambda =\n    \\(Just a) { b } _ ->\n        a\n",
    );
    assert_renders(
        "lambda2 = \\x {- c -} -> x\n",
        "lambda2 =\n    \\x ->\n        {- c -}\n        x\n",
    );
}

#[test]
fn ifs_stack_their_clauses() {
    assert_renders(
        "ifs = if a then b else if {- c -} c then d else\n  -- e\n  e\n",
        "ifs =\n    if a then\n        b\n\n    else if {- c -} c then\n        d\n\n    else\n        -- e\n        e\n",
    );
}

#[test]
fn lets_space_definitions_but_not_annotations() {
    assert_renders(
        "lets = let\n  -- leading\n  x : Int\n  x = 1\n  (y, z) = (2, 3)\n  f a = a\n  in\n  -- body\n  x + y\n",
        "lets =\n    let\n        -- leading\n        x : Int\n        x =\n            1\n\n        ( y, z ) =\n            ( 2, 3 )\n\n        f a =\n            a\n    in\n    -- body\n    x + y\n",
    );
}

#[test]
fn case_branches_keep_their_comments() {
    assert_renders(
        "cases = case (x) of\n  0 -> a\n  {- p -}\n  Just y -> {- r -} y\n  _ -> -- t\n     z\n",
        "cases =\n    case x of\n        0 ->\n            a\n\n        {- p -}\n        Just y ->\n            {- r -}\n            y\n\n        _ ->\n            -- t\n            z\n",
    );
    assert_renders(
        "c = case x of\n  Just y\n    {- q -}\n    -> y\n",
        "c =\n    case x of\n        Just y\n        {- q -}\n            ->\n                y\n",
    );
}

#[test]
fn records_accessors_and_tuples() {
    assert_renders(
        "records = { r | a = 1, b = { c = (2) } }\n",
        "records =\n    { r | a = 1, b = { c = 2 } }\n",
    );
    assert_renders(
        "access = (f x).field.other\n",
        "access =\n    (f x).field.other\n",
    );
    assert_renders(
        "fns = (.field, (,), (+))\n",
        "fns =\n    ( .field, (,), (+) )\n",
    );
}

#[test]
fn ranges_become_list_range() {
    assert_renders("range = [a..b]\n", "range =\n    List.range a b\n");
}

#[test]
fn lists_keep_line_and_trailing_comments() {
    assert_renders(
        "list = [ a -- one\n  , b\n  -- trailing\n  ]\n",
        "list =\n    [ a -- one\n    , b\n\n    -- trailing\n    ]\n",
    );
}
