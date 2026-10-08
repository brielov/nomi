//! `--report=json`: a port of `Json.Encode`'s `encodeUgly` and of the
//! encoders in `Reporting/Doc.hs`, `Reporting/Error.hs` and
//! `Reporting/Exit/Help.hs`. Keys stay in upstream's order and strings are
//! escaped as `E.chars` escapes them, so the bytes match.

use std::fmt;

use super::Report;
use super::code::Region;
use super::doc::Doc;
use super::render::{self, Chunk};
use crate::Compat;

/// A JSON value, as `Json.Encode` builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(&'static str, Json)>),
}

/// `encodeUgly`: no spaces, no newlines.
impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Int(n) => write!(f, "{n}"),
            Json::Str(s) => write_str(f, s),
            Json::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Json::Object(fields) => {
                f.write_str("{")?;
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "\"{key}\":{value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

/// `E.chars`, which escapes only CR, LF, quotes and backslashes: a tab or
/// other control character goes out as it is.
fn write_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for c in s.chars() {
        match c {
            '\r' => f.write_str("\\r")?,
            '\n' => f.write_str("\\n")?,
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

fn string(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}

/// `D.encode`: an unstyled chunk as a string, a styled one as an object.
/// `doc`'s snippets must have been put in with `code::expand`.
pub fn message(doc: &Doc, compat: Compat) -> Json {
    let chunk = |Chunk { text, style }: Chunk| {
        if style.color.is_none() && !style.bold && !style.underline {
            return string(text);
        }
        Json::Object(vec![
            ("bold", Json::Bool(style.bold)),
            ("underline", Json::Bool(style.underline)),
            (
                "color",
                style.color.map_or(Json::Null, |c| string(c.json_name())),
            ),
            ("string", string(text)),
        ])
    };
    Json::Array(render::chunks(doc, compat).into_iter().map(chunk).collect())
}

/// `encodeRegion`: 0.19.1 prints positions as it keeps them, from 1;
/// 0.19.2 as it keeps them, from 0; 0.19.3 adds 1 for editors.
pub fn region(region: Region, compat: Compat) -> Json {
    let shift = i64::from(compat.first_json_position()) - i64::from(compat.first_position());
    let position = |row: u32, col: u32| {
        Json::Object(vec![
            ("line", Json::Int(i64::from(row) + shift)),
            ("column", Json::Int(i64::from(col) + shift)),
        ])
    };
    Json::Object(vec![
        ("start", position(region.start.row, region.start.col)),
        ("end", position(region.end.row, region.end.col)),
    ])
}

/// `Error.reportToJson`: one problem in a module.
pub fn problem(title: &str, at: Region, message_doc: &Doc, compat: Compat) -> Json {
    Json::Object(vec![
        ("title", string(title)),
        ("region", region(at, compat)),
        ("message", message(message_doc, compat)),
    ])
}

/// `Error.toJson`: a module's problems.
pub fn module(path: &str, name: &str, problems: Vec<Json>) -> Json {
    Json::Object(vec![
        ("path", string(path)),
        ("name", string(name)),
        ("problems", Json::Array(problems)),
    ])
}

/// `Help.reportToJson` for `CompilerReport`: every module's problems.
pub fn compile_errors(modules: Vec<Json>) -> Json {
    Json::Object(vec![
        ("type", string("compile-errors")),
        ("errors", Json::Array(modules)),
    ])
}

/// `Help.reportToJson` for a `Report`.
pub fn report(report: &Report, compat: Compat) -> Json {
    Json::Object(vec![
        ("type", string("error")),
        ("path", report.path.as_deref().map_or(Json::Null, string)),
        ("title", string(report.title.as_ref())),
        ("message", message(&report.message, compat)),
    ])
}

#[cfg(test)]
mod tests;
