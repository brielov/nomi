//! JSON with its objects' fields in the order they came, which
//! `serde_json::Value` sorts: the `JUnit` reporter's description of its XML,
//! and `elm.json` as elm-test reads and writes it.

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

impl Json {
    /// The field `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The field `key` of an object, to change.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(fields) => fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets the field `key` of an object, where it is or, new, last, as
    /// JavaScript's assignment does.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Object(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(field) => field.1 = value,
                None => fields.push((key.to_owned(), value)),
            }
        }
    }

    /// Removes the field `key` of an object, as `delete` does.
    pub fn remove(&mut self, key: &str) -> Option<Json> {
        match self {
            Json::Object(fields) => {
                let at = fields.iter().position(|(k, _)| k == key)?;
                Some(fields.remove(at).1)
            }
            _ => None,
        }
    }

    /// The names of an object's fields, in order.
    pub fn keys(&self) -> Vec<String> {
        match self {
            Json::Object(fields) => fields.iter().map(|(k, _)| k.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// `JSON.stringify(value, null, 4)`.
    pub fn stringify(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let indent = |out: &mut String, depth: usize| out.push_str(&" ".repeat(4 * depth));
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(&b.to_string()),
            Json::Number(n) => out.push_str(&n.to_string()),
            Json::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
            Json::Array(items) if items.is_empty() => out.push_str("[]"),
            Json::Object(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    out.push_str(if i == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    out.push_str(&serde_json::to_string(key).unwrap_or_default());
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_stringified_as_javascript_does() {
        let text = "{\"b\":[1,2.5,{}],\"a\":{\"x\":\"q\\\"\\u001f\",\"e\":[]},\"n\":null}";
        let json: Json = serde_json::from_str(text).unwrap();
        assert_eq!(json.keys(), ["b", "a", "n"]);
        assert_eq!(
            json.stringify(),
            "{\n    \"b\": [\n        1,\n        2.5,\n        {}\n    ],\n    \"a\": {\n        \"x\": \"q\\\"\\u001f\",\n        \"e\": []\n    },\n    \"n\": null\n}"
        );
    }
}
