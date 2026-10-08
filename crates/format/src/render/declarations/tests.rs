//! Expected outputs are elm-format 0.8.8's, for whole modules.

#[track_caller]
fn assert_formats(text: &str, expected: &str) {
    assert_eq!(crate::format(text).as_deref(), Some(expected), "{text}");
}

#[test]
fn declarations_and_comments_take_their_spacing() {
    assert_formats(
        "port module M exposing (..)\ntype Msg = A | B Int (List String)\n  | C {- c -} Int\ntype alias R a = { x : a }\ntype alias {- a -} Long = \n  -- why\n  Int\nport out : String -> Cmd msg\nf : Int\nf = 1\ng = 2\n-- comment\nh = 3\n\n\n{-| docs -}\nk = 4\n{--}\nx = 1\n--}\ny = 2\n(+++) : a -> a\n(+++) a = a\n",
        "port module M exposing (..)\n\n\ntype Msg\n    = A\n    | B Int (List String)\n    | C {- c -} Int\n\n\ntype alias R a =\n    { x : a }\n\n\ntype alias {- a -} Long =\n    -- why\n    Int\n\n\nport out : String -> Cmd msg\n\n\nf : Int\nf =\n    1\n\n\ng =\n    2\n\n\n\n-- comment\n\n\nh =\n    3\n\n\n{-| docs\n-}\nk =\n    4\n\n\n{--}\nx =\n    1\n--}\n\n\ny =\n    2\n\n\n(+++) : a -> a\n(+++) a =\n    a\n",
    );
}

#[test]
fn fixities_stay_together_and_type_heads_can_break() {
    assert_formats(
        "module M exposing (..)\nimport Basics exposing (add)\ninfix left  6 (+) = add\ninfix right 5 (++) = append\ninfix non   4 (==) = eq\ntype T a -- why\n  b = T a\n  | U\n    -- arg\n    b\n",
        "module M exposing (..)\n\nimport Basics exposing (add)\n\n\ninfix left  6 (+) = add\ninfix right 5 (++) = append\ninfix non   4 (==) = eq\n\n\ntype\n    T\n        a\n        -- why\n        b\n    = T a\n    | U\n        -- arg\n        b\n",
    );
}

#[test]
fn elm_0_18_fixities_print_on_one_line() {
    assert_formats(
        "module M exposing (..)\nx = 1\n{-| doc -}\ninfixl 4 +++\ninfixr\n  5 {- a -}\n  ***\ninfix 9 `M.op`\ninfixl 0 .\ny = 2\n",
        "module M exposing (..)\n\n\nx =\n    1\n\n\n{-| doc\n-}\n\n\ninfixl 4 +++\ninfixr 5 {- a -} ***\ninfix 9 `M.op`\ninfixl 0 .\n\n\ny =\n    2\n",
    );
}

#[test]
fn a_fixity_keeps_its_doc_comment() {
    assert_formats(
        "module M exposing (..)\nx = 1\n{-| doc -}\ninfix left 4 (+++) = f\ny = 2\n",
        "module M exposing (..)\n\n\nx =\n    1\n\n\n{-| doc\n-}\n\n\ninfix left  4 (+++) = f\n\n\ny =\n    2\n",
    );
}

#[test]
fn elm_0_16_port_definitions_print_as_definitions() {
    assert_formats(
        "module A exposing (..)\nx = 1\nport out = signal\n{-| doc -}\nport tasks {- a -} = -- b\n    Signal.map f\n        inputs\nport tasks : Signal Int\nport z\n  {- c -}\n  = 1\nport w =\n  {- d -} 2\ny = 2\n",
        "module A exposing (..)\n\n\nx =\n    1\n\n\nport out =\n    signal\n\n\n{-| doc\n-}\nport tasks {- a -} =\n    -- b\n    Signal.map f\n        inputs\nport tasks : Signal Int\n\n\nport z {- c -} =\n    1\n\n\nport w =\n    {- d -} 2\n\n\ny =\n    2\n",
    );
}

#[test]
fn a_module_without_a_header_exposes_no_port_definition() {
    let formatted = crate::format("port out = signal\nport inp : Signal Int\nx = 1\n").unwrap();
    assert_eq!(
        formatted.lines().next(),
        Some("module Main exposing (inp, x)")
    );
}
