use super::*;
use crate::diagnostic::doc::{Color, text};

#[test]
fn every_change_of_style_ends_a_chunk_even_an_empty_one() {
    let doc = text("a").append(text("b").color(Color::Red)).underline();
    assert_eq!(
        message(&doc, Compat::LATEST).to_string(),
        concat!(
            r#"["","#,
            r#"{"bold":false,"underline":true,"color":null,"string":"a"},"#,
            r#"{"bold":false,"underline":true,"color":"RED","string":"b"},"#,
            r#"{"bold":false,"underline":true,"color":null,"string":""},"#,
            r#"""]"#,
        )
    );
}

#[test]
fn only_cr_lf_quotes_and_backslashes_are_escaped() {
    let json = Json::Str("\r\n\"\\\t\u{1}é".into());
    assert_eq!(json.to_string(), "\"\\r\\n\\\"\\\\\t\u{1}é\"");
}

#[test]
fn a_strict_string_escapes_every_control_character() {
    assert_eq!(
        strict_string("\r\n\"\\\t\u{1}\u{1f} é\u{7f}"),
        "\"\\r\\n\\\"\\\\\\t\\u0001\\u001f é\u{7f}\""
    );
}

#[test]
fn a_report_without_a_path_has_a_null_one() {
    let report = Report::new("NO ELM.JSON", None, "Nothing here.", []);
    assert_eq!(
        super::report(&report, Compat::LATEST).to_string(),
        r#"{"type":"error","path":null,"title":"NO ELM.JSON","message":["Nothing here."]}"#
    );
}
