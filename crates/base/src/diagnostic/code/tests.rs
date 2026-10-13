//! Reports with snippets against what each release printed for them, as
//! text and as `--report=json`.

use text_size::{TextRange, TextSize};

use super::*;
use crate::diagnostic::Report;
use crate::diagnostic::doc::{self, hint, reflow, stack};
use crate::diagnostic::json::{compile_errors, module, problem};
use crate::diagnostic::render::{Styles, WIDTH, render};

/// A report on `A.elm` and what each release, oldest first, printed for it.
struct Case {
    source: &'static str,
    name: &'static str,
    title: &'static str,
    region: TextRange,
    message: Doc,
    text: [&'static str; 3],
    json: [&'static str; 3],
}

fn check(case: &Case) {
    let source = Source::new(case.source);
    for (i, compat) in Compat::ALL.into_iter().enumerate() {
        let message = expand(&case.message, &source, compat);
        let report = Report::doc(case.title, Some("A.elm".into()), message.clone(), []);
        let text = render(&report.to_doc(), compat, WIDTH, Styles::Plain);
        assert_eq!(text, case.text[i], "{compat}");
        let region = source.region(case.region, compat);
        let problem = problem(case.title, region, &message, compat);
        let json = compile_errors(vec![module("A.elm", case.name, vec![problem])]);
        assert_eq!(json.to_string(), case.json[i], "{compat}");
    }
}

/// The range of the `nth` `needle` in `source`, from 0.
fn find(source: &str, needle: &str, nth: usize) -> TextRange {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .expect("in the source");
    TextRange::at(TextSize::new(start as u32), TextSize::of(needle))
}

fn redundant_export(source: &'static str, text: [&'static str; 3], json: [&'static str; 3]) {
    let (first, second) = (find(source, "a", 0), find(source, "a", 1));
    let message = doc::pair(
        first,
        second,
        (
            reflow("You are trying to expose `a` multiple times!"),
            reflow("Remove one of them and you should be all set!"),
        ),
        (
            reflow("You are trying to expose `a` multiple times! Once here:"),
            reflow("And again right here:"),
            reflow("Remove one of them and you should be all set!"),
        ),
    );
    check(&Case {
        source,
        name: "A",
        title: "REDUNDANT EXPORT",
        region: second,
        message,
        text,
        json,
    });
}

#[test]
fn a_pair_on_one_row_is_numbered_as_kept() {
    redundant_export(
        PAIR1_SOURCE,
        [PAIR1_TXT_1, PAIR1_TXT_2, PAIR1_TXT_3],
        [PAIR1_JSON_1, PAIR1_JSON_2, PAIR1_JSON_3],
    );
}

#[test]
fn a_pair_on_two_rows_is_two_snippets() {
    redundant_export(
        PAIR2_SOURCE,
        [PAIR2_TXT_1, PAIR2_TXT_2, PAIR2_TXT_3],
        [PAIR2_JSON_1, PAIR2_JSON_2, PAIR2_JSON_3],
    );
}

#[test]
fn a_region_over_rows_marks_them() {
    let start = find(MULTI_SOURCE, "\"x\"", 0).start();
    let end = find(MULTI_SOURCE, "\"y\"", 0).end();
    let region = TextRange::new(start, end);
    let found = |name: &str| text(name).color(Color::DullYellow).indent(4);
    let after = stack([
        reflow("The body is:"),
        found("String"),
        reflow("But the type annotation on `a` says it should be:"),
        found("Int"),
        hint(
            "Want to convert a String into an Int? Use the"
                .split(' ')
                .map(text)
                .chain([text("String.toInt").color(Color::Green), text("function!")]),
        ),
    ]);
    check(&Case {
        source: MULTI_SOURCE,
        name: "A",
        title: "TYPE MISMATCH",
        region,
        message: doc::snippet(
            reflow("Something is off with the body of the `a` definition:"),
            region,
            None,
            after,
        ),
        text: [MULTI_TXT_1, MULTI_TXT_2, MULTI_TXT_3],
        json: [MULTI_JSON_1, MULTI_JSON_2, MULTI_JSON_3],
    });
}

#[test]
fn an_empty_highlight_gets_one_caret_and_tabs_stay_raw() {
    let region = TextRange::empty(find(TAB_SOURCE, "\t", 0).start());
    check(&Case {
        source: TAB_SOURCE,
        name: "???",
        title: "NO TABS",
        region,
        message: doc::snippet(
            reflow("I ran into a tab, but tabs are not allowed in Elm files."),
            region,
            None,
            reflow("Replace the tab with spaces."),
        ),
        text: [TAB_TXT_1, TAB_TXT_2, TAB_TXT_3],
        json: [TAB_JSON_1, TAB_JSON_2, TAB_JSON_3],
    });
}

#[test]
fn crlf_keeps_its_cr_in_the_line() {
    let source = "a =\r\n    b\r\n";
    let range = find(source, "b", 0);
    let doc = expand(
        &Doc::snippet(range, range),
        &Source::new(source),
        Compat::V0_19_1,
    );
    let text = render(&doc, Compat::V0_19_1, WIDTH, Styles::Plain);
    assert_eq!(text, "2|     b\r\n       ^");
}

// What `elm make` and `elm make --report=json` printed for
// `cargo xtask oracle`'s releases: the source, then each release's text
// and JSON.
const PAIR1_SOURCE: &str = "module A exposing (a, a)\n\n\na =\n    1\n";
const PAIR1_TXT_1: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times!\n\n1| module A exposing (a, a)\n                      ^  ^\nRemove one of them and you should be all set!\n\n";
const PAIR1_JSON_1: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":1,\"column\":23},\"end\":{\"line\":1,\"column\":24}},\"message\":[\"You are trying to expose `a` multiple times!\\n\\n1| module A exposing (a, a)\\n                      \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"  \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const PAIR1_TXT_2: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times!\n\n0| module A exposing (a, a)\n                      ^  ^\nRemove one of them and you should be all set!\n\n";
const PAIR1_JSON_2: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":0,\"column\":22},\"end\":{\"line\":0,\"column\":23}},\"message\":[\"You are trying to expose `a` multiple times!\\n\\n0| module A exposing (a, a)\\n                      \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"  \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const PAIR1_TXT_3: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times!\n\n0| module A exposing (a, a)\n                      ^  ^\nRemove one of them and you should be all set!\n\n";
const PAIR1_JSON_3: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":1,\"column\":23},\"end\":{\"line\":1,\"column\":24}},\"message\":[\"You are trying to expose `a` multiple times!\\n\\n0| module A exposing (a, a)\\n                      \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"  \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const PAIR2_SOURCE: &str = "module A exposing\n    ( a\n    , a\n    )\n\n\na =\n    1\n";
const PAIR2_TXT_1: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times! Once here:\n\n2|     ( a\n         ^\nAnd again right here:\n\n3|     , a\n         ^\nRemove one of them and you should be all set!\n\n";
const PAIR2_JSON_1: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":3,\"column\":7},\"end\":{\"line\":3,\"column\":8}},\"message\":[\"You are trying to expose `a` multiple times! Once here:\\n\\n2|     ( a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nAnd again right here:\\n\\n3|     , a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const PAIR2_TXT_2: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times! Once here:\n\n2|     ( a\n         ^\nAnd again right here:\n\n3|     , a\n         ^\nRemove one of them and you should be all set!\n\n";
const PAIR2_JSON_2: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":2,\"column\":6},\"end\":{\"line\":2,\"column\":7}},\"message\":[\"You are trying to expose `a` multiple times! Once here:\\n\\n2|     ( a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nAnd again right here:\\n\\n3|     , a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const PAIR2_TXT_3: &str = "-- REDUNDANT EXPORT ------------------------------------------------------ A.elm\n\nYou are trying to expose `a` multiple times! Once here:\n\n2|     ( a\n         ^\nAnd again right here:\n\n3|     , a\n         ^\nRemove one of them and you should be all set!\n\n";
const PAIR2_JSON_3: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"REDUNDANT EXPORT\",\"region\":{\"start\":{\"line\":3,\"column\":7},\"end\":{\"line\":3,\"column\":8}},\"message\":[\"You are trying to expose `a` multiple times! Once here:\\n\\n2|     ( a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nAnd again right here:\\n\\n3|     , a\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nRemove one of them and you should be all set!\"]}]}]}";
const MULTI_SOURCE: &str = "module A exposing (a)\n\n\na : Int\na =\n    \"x\"\n        ++ \"y\"\n";
const MULTI_TXT_1: &str = "-- TYPE MISMATCH --------------------------------------------------------- A.elm\n\nSomething is off with the body of the `a` definition:\n\n6|>    \"x\"\n7|>        ++ \"y\"\n\nThe body is:\n\n    String\n\nBut the type annotation on `a` says it should be:\n\n    Int\n\nHint: Want to convert a String into an Int? Use the String.toInt function!\n\n";
const MULTI_JSON_1: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"TYPE MISMATCH\",\"region\":{\"start\":{\"line\":6,\"column\":5},\"end\":{\"line\":7,\"column\":15}},\"message\":[\"Something is off with the body of the `a` definition:\\n\\n6|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"    \\\"x\\\"\\n7|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"        ++ \\\"y\\\"\\n\\nThe body is:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"String\"},\"\\n\\nBut the type annotation on `a` says it should be:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"Int\"},\"\\n\\n\",{\"bold\":false,\"underline\":true,\"color\":null,\"string\":\"Hint\"},\": Want to convert a String into an Int? Use the \",{\"bold\":false,\"underline\":false,\"color\":\"GREEN\",\"string\":\"String.toInt\"},\" function!\"]}]}]}";
const MULTI_TXT_2: &str = "-- TYPE MISMATCH --------------------------------------------------------- A.elm\n\nSomething is off with the body of the `a` definition:\n\n6|>    \"x\"\n7|>        ++ \"y\"\n\nThe body is:\n\n    String\n\nBut the type annotation on `a` says it should be:\n\n    Int\n\nHint: Want to convert a String into an Int? Use the String.toInt function!\n\n";
const MULTI_JSON_2: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"TYPE MISMATCH\",\"region\":{\"start\":{\"line\":5,\"column\":4},\"end\":{\"line\":6,\"column\":14}},\"message\":[\"Something is off with the body of the `a` definition:\\n\\n6|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"    \\\"x\\\"\\n7|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"        ++ \\\"y\\\"\\n\\nThe body is:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"String\"},\"\\n\\nBut the type annotation on `a` says it should be:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"Int\"},\"\\n\\n\",{\"bold\":false,\"underline\":true,\"color\":null,\"string\":\"Hint\"},\": Want to convert a String into an Int? Use the \",{\"bold\":false,\"underline\":false,\"color\":\"GREEN\",\"string\":\"String.toInt\"},\" function!\"]}]}]}";
const MULTI_TXT_3: &str = "-- TYPE MISMATCH --------------------------------------------------------- A.elm\n\nSomething is off with the body of the `a` definition:\n\n6|>    \"x\"\n7|>        ++ \"y\"\n\nThe body is:\n\n    String\n\nBut the type annotation on `a` says it should be:\n\n    Int\n\nHint: Want to convert a String into an Int? Use the String.toInt function!\n\n";
const MULTI_JSON_3: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"A\",\"problems\":[{\"title\":\"TYPE MISMATCH\",\"region\":{\"start\":{\"line\":6,\"column\":5},\"end\":{\"line\":7,\"column\":15}},\"message\":[\"Something is off with the body of the `a` definition:\\n\\n6|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"    \\\"x\\\"\\n7|\",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\">\"},\"        ++ \\\"y\\\"\\n\\nThe body is:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"String\"},\"\\n\\nBut the type annotation on `a` says it should be:\\n\\n    \",{\"bold\":false,\"underline\":false,\"color\":\"yellow\",\"string\":\"Int\"},\"\\n\\n\",{\"bold\":false,\"underline\":true,\"color\":null,\"string\":\"Hint\"},\": Want to convert a String into an Int? Use the \",{\"bold\":false,\"underline\":false,\"color\":\"GREEN\",\"string\":\"String.toInt\"},\" function!\"]}]}]}";
const TAB_SOURCE: &str = "module A exposing (a)\n\n\na =\n    [ 1\n    , \t2\n    ]\n";
const TAB_TXT_1: &str = "-- NO TABS --------------------------------------------------------------- A.elm\n\nI ran into a tab, but tabs are not allowed in Elm files.\n\n6|     , \t2\n         ^\nReplace the tab with spaces.\n\n";
const TAB_JSON_1: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"???\",\"problems\":[{\"title\":\"NO TABS\",\"region\":{\"start\":{\"line\":6,\"column\":7},\"end\":{\"line\":6,\"column\":7}},\"message\":[\"I ran into a tab, but tabs are not allowed in Elm files.\\n\\n6|     , \t2\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nReplace the tab with spaces.\"]}]}]}";
const TAB_TXT_2: &str = "-- NO TABS --------------------------------------------------------------- A.elm\n\nI ran into a tab, but tabs are not allowed in Elm files.\n\n6|     , \t2\n         ^\nReplace the tab with spaces.\n\n";
const TAB_JSON_2: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"???\",\"problems\":[{\"title\":\"NO TABS\",\"region\":{\"start\":{\"line\":5,\"column\":6},\"end\":{\"line\":5,\"column\":6}},\"message\":[\"I ran into a tab, but tabs are not allowed in Elm files.\\n\\n6|     , \t2\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nReplace the tab with spaces.\"]}]}]}";
const TAB_TXT_3: &str = "-- NO TABS --------------------------------------------------------------- A.elm\n\nI ran into a tab, but tabs are not allowed in Elm files.\n\n6|     , \t2\n         ^\nReplace the tab with spaces.\n\n";
const TAB_JSON_3: &str = "{\"type\":\"compile-errors\",\"errors\":[{\"path\":\"A.elm\",\"name\":\"???\",\"problems\":[{\"title\":\"NO TABS\",\"region\":{\"start\":{\"line\":6,\"column\":7},\"end\":{\"line\":6,\"column\":7}},\"message\":[\"I ran into a tab, but tabs are not allowed in Elm files.\\n\\n6|     , \t2\\n         \",{\"bold\":false,\"underline\":false,\"color\":\"RED\",\"string\":\"^\"},\"\\nReplace the tab with spaces.\"]}]}]}";
