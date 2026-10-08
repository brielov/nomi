use super::*;
use crate::diagnostic::doc::{Color, text};

#[test]
fn every_change_of_style_ends_a_chunk_even_an_empty_one() {
    let doc = text("a").append(text("b").color(Color::Red)).bold();
    assert_eq!(
        message(&doc, Compat::LATEST).to_string(),
        concat!(
            r#"["","#,
            r#"{"bold":true,"underline":false,"color":null,"string":"a"},"#,
            r#"{"bold":true,"underline":false,"color":"RED","string":"b"},"#,
            r#"{"bold":true,"underline":false,"color":null,"string":""},"#,
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
fn a_report_without_a_path_has_a_null_one() {
    let report = Report::new("NO ELM.JSON", None, "Nothing here.", []);
    assert_eq!(
        super::report(&report, Compat::LATEST).to_string(),
        r#"{"type":"error","path":null,"title":"NO ELM.JSON","message":["Nothing here."]}"#
    );
}
