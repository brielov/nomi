//! The `JUnit` report's XML (`XMLBuilder.toString`): the `JUnit` reporter's
//! summary, a JSON description of the XML, with the results added to its
//! test cases, written as elm-test writes it. Fields come in the order
//! the reporter encoded them, which [`Json`] keeps.

use std::fmt::Write;

use crate::json::Json;

/// The `JUnit` reporter's `summary` with `results` added to its test
/// suite's test cases, as XML.
pub(crate) fn report(mut summary: Json, results: Vec<Json>) -> String {
    if let Some(suite) = summary.get_mut("testsuite") {
        match suite.get_mut("testcase") {
            Some(Json::Array(cases)) => cases.extend(results),
            Some(other) => {
                let mut cases = vec![std::mem::replace(other, Json::Null)];
                cases.extend(results);
                *other = Json::Array(cases);
            }
            None => suite.set("testcase", Json::Array(results)),
        }
    }
    to_string(&summary)
}

/// `toString`.
fn to_string(xml: &Json) -> String {
    let mut out = String::from("<?xml version=\"1.0\"?>");
    if let Json::Object(fields) = xml {
        for (key, value) in fields {
            element(&mut out, key, value);
        }
    }
    out
}

/// `xmlValueToString`.
fn element(out: &mut String, name: &str, value: &Json) {
    if let Json::Array(items) = value {
        for item in items {
            element(out, name, item);
        }
        return;
    }
    let mut attributes = String::new();
    let mut children = String::new();
    match value {
        Json::Object(fields) => {
            for (key, value) in fields {
                match key.strip_prefix('@') {
                    Some(attribute) => {
                        _ = write!(attributes, " {attribute}=\"{}\"", attribute_value(value));
                    }
                    None => element(&mut children, key, value),
                }
            }
        }
        Json::String(text) => children = escape_text(text),
        other => children = scalar(other),
    }
    if children.is_empty() {
        _ = write!(out, "<{name}{attributes}/>");
    } else {
        _ = write!(out, "<{name}{attributes}>{children}</{name}>");
    }
}

/// `xmlValueToAttributeValue`, for the strings and numbers it takes.
fn attribute_value(value: &Json) -> String {
    match value {
        Json::String(text) => escape_attribute(text),
        other => scalar(other),
    }
}

/// What JavaScript's `toString` makes of a number, a boolean or null.
fn scalar(value: &Json) -> String {
    match value {
        Json::Number(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Null => "null".to_owned(),
        Json::String(s) => s.clone(),
        Json::Array(_) | Json::Object(_) => String::new(),
    }
}

/// `invalidCharRegex`'s characters as `\u{…}`. Rust's strings hold no
/// lone surrogates, its other alternatives.
fn replace_invalid(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let invalid = matches!(c, '\0'..='\x08' | '\x0B' | '\x0C' | '\x0E'..='\x1F' | '\u{FFFE}' | '\u{FFFF}');
        if invalid {
            _ = write!(out, "\\u{{{:04x}}}", c as u32);
        } else {
            out.push(c);
        }
    }
    out
}

/// `escapeAttributeValue`.
fn escape_attribute(text: &str) -> String {
    replace_invalid(text)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
        .replace('\t', "&#x9;")
        .replace('\n', "&#xA;")
        .replace('\r', "&#xD;")
}

/// `escapeText`.
fn escape_text(text: &str) -> String {
    replace_invalid(text)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#xD;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_is_written_as_elm_tests() {
        let summary: Json = serde_json::from_str(
            r#"{"testsuite":{"@name":"elm-test","@tests":2,"@time":0.5,"testcase":[]}}"#,
        )
        .unwrap();
        let case = |json: &str| serde_json::from_str::<Json>(json).unwrap();
        let results = vec![
            case(r#"{"@classname":"A b","@name":"is \"q\"\n","@time":"0"}"#),
            case(r#"{"@classname":"A","@name":"c","@time":"0","failure":"1 < 2 & \u001b\r"}"#),
        ];
        assert_eq!(
            report(summary, results),
            "<?xml version=\"1.0\"?><testsuite name=\"elm-test\" tests=\"2\" time=\"0.5\">\
             <testcase classname=\"A b\" name=\"is &quot;q&quot;&#xA;\" time=\"0\"/>\
             <testcase classname=\"A\" name=\"c\" time=\"0\"><failure>1 &lt; 2 &amp; \\u{001b}&#xD;</failure></testcase>\
             </testsuite>"
        );
    }
}
