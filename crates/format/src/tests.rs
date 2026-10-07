use super::*;

#[test]
fn rejects_what_elm_format_rejects_and_keeps_formatted_modules() {
    assert_eq!(format("module Main exposing (x)\n\nx =\n"), None);
    let text = "module Main exposing (x)\n\n\nx =\n    (<|)\n";
    assert_eq!(format(text).as_deref(), Some(text));
}
