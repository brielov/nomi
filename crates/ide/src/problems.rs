//! A project's problems as elm-language-server publishes them
//! (DECISIONS.md 2026-10-12): elm's own reports, in its words, where its
//! `elm make` diagnostics would put them, and the annotations a module
//! leaves out, as its type checker suggests them.
//!
//! It writes a report's message as one line of text, its title first:
//! `TITLE - ` and the message, each styled part between `#`s, and keys
//! its code actions on a code taken from the title. A problem's region
//! counts columns in code points, which this turns into the file's byte
//! offsets, so a protocol position comes out right past any character.

use std::path::{Path, PathBuf};

use nomi_base::diagnostic::json::{self, Json};
use nomi_base::diagnostic::{Problem as Report, Report as ProjectReport};
use nomi_base::{Compat, TextRange};
use nomi_compiler::hir::Def;
use nomi_compiler::types;
use nomi_compiler::types::render::{self, Context};
use nomi_engine::Snapshot;
use nomi_engine::check::Checked;
use nomi_engine::index::Index;
use nomi_engine::live::Live;

use crate::elm_offset;
use crate::hover::localizer;

/// How much a problem matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
}

/// A problem in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: PathBuf,
    pub range: TextRange,
    pub severity: Severity,
    pub message: String,
    /// Who says so: `Elm` for elm's reports and the annotations,
    /// `ElmLS` for the lint rules.
    pub source: &'static str,
    /// What the problem is, for code actions: `missing_value`,
    /// `import_resolve`, `elm_make` and so on.
    pub code: &'static str,
    /// Whether it marks code nothing needs, which editors fade.
    pub unnecessary: bool,
    /// The file as the problem's data names it: relative to the project
    /// for elm's reports, as elm-language-server keeps them; none for its
    /// URI.
    pub relative: Option<String>,
    /// The edits that fix it, which elm-review gives.
    pub fixes: Vec<crate::Edit>,
}

/// The problems elm reports in the project `live` checked: each module's,
/// or the project's own, on each of the `open` files.
pub fn compiler_problems(snapshot: &Snapshot, live: &Live, open: &[PathBuf]) -> Vec<Problem> {
    let relative = |path: &Path| {
        let path = path.strip_prefix(&live.root).unwrap_or(path);
        let parts: Vec<_> = path.iter().map(|p| p.to_string_lossy()).collect();
        parts.join("/")
    };
    match &live.checked {
        Checked::Fine => Vec::new(),
        Checked::Project(report) => {
            let message = project_message(report, live.compat);
            (open.iter())
                .map(|path| Problem {
                    path: path.clone(),
                    range: TextRange::default(),
                    severity: Severity::Error,
                    message: message.clone(),
                    source: "Elm",
                    code: code(&report.title),
                    unnecessary: false,
                    relative: Some(relative(path)),
                    fixes: Vec::new(),
                })
                .collect()
        }
        Checked::Modules(modules) => {
            let mut problems = Vec::new();
            for module in modules {
                let path = live.root.join(&module.path);
                let Ok(text) = snapshot.read(&path) else {
                    continue;
                };
                for report in &module.problems {
                    problems.push(Problem {
                        path: path.clone(),
                        range: range(&text, report, live.compat),
                        severity: Severity::Error,
                        message: module_message(report, live.compat),
                        source: "Elm",
                        code: code(&report.title),
                        unnecessary: false,
                        relative: Some(relative(&path)),
                        fixes: Vec::new(),
                    });
                }
            }
            problems
        }
    }
}

/// `elmDiagnosticsHelper`'s code for a report's title.
fn code(title: &str) -> &'static str {
    if title.starts_with("NAMING ERROR") {
        "missing_value"
    } else if title.starts_with("MODULE NOT FOUND") {
        "import_resolve"
    } else {
        // It also means to give `MISSING PATTERNS` its own code, but tests
        // the title for text that comes after it in the message.
        "elm_make"
    }
}

fn range(text: &str, report: &Report, compat: Compat) -> TextRange {
    let (start, end) = (report.region.start, report.region.end);
    let start = elm_offset(text, start.row, start.col, compat);
    let end = elm_offset(text, end.row, end.col, compat).max(start);
    TextRange::new(start, end)
}

/// A module's problem: the title, then the message with each styled
/// part between `#`s.
fn module_message(report: &Report, compat: Compat) -> String {
    let mut details = String::new();
    for part in parts(&json::message(&report.message, compat)) {
        match part {
            Part::Plain(text) => details.push_str(text),
            Part::Styled(text) => {
                details.push('#');
                details.push_str(text);
                details.push('#');
            }
        }
    }
    format!("{} - {}", report.title, strip_escapes(&details))
}

/// A project's problem: the title, then the message's text.
fn project_message(report: &ProjectReport, compat: Compat) -> String {
    let mut details = String::new();
    for part in parts(&json::message(&report.message, compat)) {
        match part {
            Part::Plain(text) | Part::Styled(text) => details.push_str(text),
        }
    }
    format!("{} - {}", report.title, strip_escapes(&details))
}

enum Part<'j> {
    Plain(&'j str),
    Styled(&'j str),
}

/// The text of a message as elm's JSON report gives it.
fn parts(message: &Json) -> Vec<Part<'_>> {
    let Json::Array(items) = message else {
        return Vec::new();
    };
    (items.iter())
        .filter_map(|item| match item {
            Json::Str(text) => Some(Part::Plain(text)),
            Json::Object(fields) => fields.iter().find_map(|(key, value)| match value {
                Json::Str(text) if *key == "string" => Some(Part::Styled(text)),
                _ => None,
            }),
            _ => None,
        })
        .collect()
}

/// `details.replace(/\[\d+m/g, "")`: what's left of terminal colors.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && after[digits..].starts_with('m') {
            rest = &after[digits + 1..];
        } else {
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The top-level values of the module at `path` that have no annotation,
/// each with the one inference gives it, where it parses and its types
/// check.
pub fn missing_annotations(index: &Index, path: &Path) -> Vec<Problem> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    // A module that doesn't parse has its syntax error to show, and its
    // recovered tree's types are no one's.
    if !nomi_syntax::parse_module(&module.text).errors().is_empty() {
        return Vec::new();
    }
    let Some(canonical) = &module.canonical else {
        return Vec::new();
    };
    let (Ok(inferred), _) = types::infer_types(index.compat, canonical) else {
        return Vec::new();
    };
    let localizer = localizer(index, module);
    let mut problems = Vec::new();
    for &id in &module.hir.values {
        let Def::Define { name, .. } = &module.hir.defs[id] else {
            continue;
        };
        if canonical.annotations.contains_key(&name.text) {
            continue;
        }
        let Some(annotation) = inferred.get(&name.text) else {
            continue;
        };
        let ty = render::can_to_doc(&localizer, Context::None, &annotation.ty).plain();
        problems.push(Problem {
            path: path.to_owned(),
            range: name.range,
            severity: Severity::Information,
            message: format!("Missing type annotation: `{ty}`."),
            source: "Elm",
            code: "missing_type_annotation",
            unnecessary: false,
            relative: None,
            fixes: Vec::new(),
        });
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_remnants_are_stripped() {
        assert_eq!(strip_escapes("a[31mred[0m [x] [12"), "ared [x] [12");
    }
}
