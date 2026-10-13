//! The JUnit report's XML (`XMLBuilder.toString`): the JUnit reporter's
//! summary, a JSON description of the XML, with the results added to its
//! test cases, written as elm-test writes it. Fields come in the order
//! the reporter encoded them, which `serde_json::Value` doesn't keep.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// A JSON value with its objects' fields in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E: de::Error>(self, b: bool) -> Result<Json, E> {
        Ok(Json::Bool(b))
    }

    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Json, E> {
        Ok(Json::Number(n.into()))
    }

    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Json, E> {
        Ok(Json::Number(n.into()))
    }

    fn visit_f64<E: de::Error>(self, n: f64) -> Result<Json, E> {
        Ok(serde_json::Number::from_f64(n).map_or(Json::Null, Json::Number))
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<Json, E> {
        Ok(Json::String(s.to_owned()))
    }

    fn visit_string<E: de::Error>(self, s: String) -> Result<Json, E> {
        Ok(Json::String(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut fields: Vec<(String, Json)> = Vec::new();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            // A repeated key keeps its first place and its last value, as
            // `JSON.parse` keeps them.
            match fields.iter_mut().find(|(k, _)| *k == key) {
                Some(field) => field.1 = value,
                None => fields.push((key, value)),
            }
        }
        Ok(Json::Object(fields))
    }
}

/// The JUnit reporter's `summary` with `results` added to its test
/// suite's test cases, as XML.
pub fn report(mut summary: Json, results: Vec<Json>) -> String {
    if let Json::Object(fields) = &mut summary
        && let Some((_, Json::Object(suite))) = fields.iter_mut().find(|(k, _)| k == "testsuite")
    {
        match suite.iter_mut().find(|(k, _)| k == "testcase") {
            Some((_, Json::Array(cases))) => cases.extend(results),
            Some((_, other)) => {
                let mut cases = vec![std::mem::replace(other, Json::Null)];
                cases.extend(results);
                *other = Json::Array(cases);
            }
            None => suite.push(("testcase".to_owned(), Json::Array(results))),
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
                        attributes
                            .push_str(&format!(" {attribute}=\"{}\"", attribute_value(value)));
                    }
                    None => element(&mut children, key, value),
                }
            }
        }
        Json::String(text) => children = escape_text(text),
        other => children = scalar(other),
    }
    if children.is_empty() {
        out.push_str(&format!("<{name}{attributes}/>"));
    } else {
        out.push_str(&format!("<{name}{attributes}>{children}</{name}>"));
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
            out.push_str(&format!("\\u{{{:04x}}}", c as u32));
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
