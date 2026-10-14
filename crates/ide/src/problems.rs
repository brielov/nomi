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
use nomi_base::{Compat, Doc, LineIndex, TextRange};
use nomi_compiler::types::render::{self, Context};
use nomi_engine::Snapshot;
use nomi_engine::check::Checked;
use nomi_engine::index::Index;
use nomi_engine::live::Live;

use crate::elm_range;
use crate::hover::localizer;

/// How much a problem matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
}

/// Who reports a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// elm, and elm-language-server's type checker.
    Elm,
    /// elm-language-server's lint rules.
    ElmLs,
    ElmReview,
}

impl Source {
    /// The source as a diagnostic names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Elm => "Elm",
            Source::ElmLs => "ElmLS",
            Source::ElmReview => "elm-review",
        }
    }

    /// The source a diagnostic names as `name`, where it is one.
    pub fn parse(name: &str) -> Option<Source> {
        [Source::Elm, Source::ElmLs, Source::ElmReview]
            .into_iter()
            .find(|source| source.as_str() == name)
    }
}

macro_rules! codes {
    ($($source:ident: [$($code:ident = $name:literal,)*])*) => {
        /// What a problem is, as elm-language-server's codes say, which
        /// its code actions key on.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Code {
            $($($code,)*)*
        }

        impl Code {
            /// The code as a diagnostic carries it.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($(Code::$code => $name,)*)*
                }
            }

            /// The code a diagnostic carries as `name`, where it is one.
            pub fn parse(name: &str) -> Option<Code> {
                match name {
                    $($($name => Some(Code::$code),)*)*
                    _ => None,
                }
            }

            /// Who reports a problem of this code.
            pub fn source(self) -> Source {
                match self {
                    $($(Code::$code)|* => Source::$source,)*
                }
            }
        }
    };
}

codes! {
    Elm: [
        MissingValue = "missing_value",
        ImportResolve = "import_resolve",
        ElmMake = "elm_make",
        MissingTypeAnnotation = "missing_type_annotation",
        RecordField = "record_field",
        IncompleteCasePattern = "incomplete_case_pattern",
    ]
    ElmReview: [
        ElmReview = "elm_review",
    ]
    ElmLs: [
        UnusedImport = "unused_import",
        UnusedImportedValue = "unused_imported_value",
        UnusedAlias = "unused_alias",
        UnusedPattern = "unused_pattern",
        UnusedTopLevel = "unused_top_level",
        UnusedIncomingPort = "unused_incoming_port",
        UnusedTypeAlias = "unused_type_alias",
        UnusedValueConstructor = "unused_value_constructor",
        MapNothingToNothing = "map_nothing_to_nothing",
        BooleanCaseExpr = "boolean_case_expr",
        DropConcatOfLists = "drop_concat_of_lists",
        DropConsOfItemAndList = "drop_cons_of_item_and_list",
        UseConsOverConcat = "use_cons_over_concat",
        SingleFieldRecord = "single_field_record",
        UnnecessaryListConcat = "unnecessary_list_concat",
        UnnecessaryPortModule = "unnecessary_port_module",
        NoUncurriedPrefix = "no_uncurried_prefix",
    ]
}

/// A problem in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: PathBuf,
    pub range: TextRange,
    pub severity: Severity,
    pub message: String,
    /// What the problem is, which says who reports it.
    pub code: Code,
    /// Whether it marks code nothing needs, which editors fade.
    pub unnecessary: bool,
    /// The file as the problem's data names it: relative to the project
    /// for elm's reports, as elm-language-server keeps them; none for its
    /// URI.
    pub relative: Option<String>,
    /// The edits that fix it, which elm-review gives.
    pub fixes: Vec<crate::Edit>,
}

impl Problem {
    /// A problem of `code` at `range` in the file at `path`, as
    /// elm-language-server reports its code: elm's an error but for a
    /// missing annotation, and its lint rules' a warning, unnecessary
    /// where the rule finds something unused.
    pub fn new(path: PathBuf, range: TextRange, code: Code, message: String) -> Problem {
        let severity = match code.source() {
            Source::Elm if code == Code::MissingTypeAnnotation => Severity::Information,
            Source::Elm => Severity::Error,
            Source::ElmLs | Source::ElmReview => Severity::Warning,
        };
        let unnecessary = matches!(
            code,
            Code::UnusedImport
                | Code::UnusedImportedValue
                | Code::UnusedAlias
                | Code::UnusedPattern
                | Code::UnusedTopLevel
                | Code::UnusedIncomingPort
                | Code::UnusedTypeAlias
                | Code::UnusedValueConstructor
        );
        Problem {
            path,
            range,
            severity,
            message,
            code,
            unnecessary,
            relative: None,
            fixes: Vec::new(),
        }
    }
}

/// The problems elm reports in the project `live` checked: each module's,
/// or the project's own, on each of the `open` files.
pub(crate) fn compiler_problems(
    snapshot: &Snapshot,
    live: &Live,
    open: &[PathBuf],
) -> Vec<Problem> {
    let problem = |path: &Path, range, title: &str, message| Problem {
        relative: Some(slashed(path.strip_prefix(&live.root).unwrap_or(path))),
        ..Problem::new(path.to_owned(), range, code(title), message)
    };
    match &live.checked {
        Checked::Fine => Vec::new(),
        Checked::Project(report) => {
            let message = message(&report.title, &report.message, Marks::Plain, live.compat);
            (open.iter())
                .map(|path| problem(path, TextRange::default(), &report.title, message.clone()))
                .collect()
        }
        Checked::Modules(modules) => {
            let mut problems = Vec::new();
            for module in modules {
                let path = live.root.join(&module.path);
                let Ok(text) = snapshot.read(&path) else {
                    continue;
                };
                let lines = LineIndex::new(&text);
                for report in &module.problems {
                    let range = elm_range(&text, &lines, report.region, live.compat);
                    let message =
                        message(&report.title, &report.message, Marks::Hashes, live.compat);
                    problems.push(problem(&path, range, &report.title, message));
                }
            }
            problems
        }
    }
}

/// `path`'s components joined with `/`, as a problem's data names files.
pub(crate) fn slashed(path: &Path) -> String {
    let parts: Vec<_> = path.iter().map(|p| p.to_string_lossy()).collect();
    parts.join("/")
}

/// `elmDiagnosticsHelper`'s code for a report's title.
fn code(title: &str) -> Code {
    if title.starts_with("NAMING ERROR") {
        Code::MissingValue
    } else if title.starts_with("MODULE NOT FOUND") {
        Code::ImportResolve
    } else {
        // It also means to give `MISSING PATTERNS` its own code, but tests
        // the title for text that comes after it in the message.
        Code::ElmMake
    }
}

/// Whether a message marks its styled parts: a module's problem does,
/// with `#`s around each, and a project's doesn't.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Marks {
    Hashes,
    Plain,
}

/// A problem's message: the title, then the message's text.
fn message(title: &str, message: &Doc, marks: Marks, compat: Compat) -> String {
    let mut details = String::new();
    let json = json::message(message, compat);
    let Json::Array(items) = &json else {
        return format!("{title} - ");
    };
    for item in items {
        match item {
            Json::Str(text) => details.push_str(text),
            Json::Object(fields) => {
                let styled = fields.iter().find_map(|(key, value)| match value {
                    Json::Str(text) if *key == "string" => Some(text),
                    _ => None,
                });
                if let Some(text) = styled {
                    let mark = if marks == Marks::Hashes { "#" } else { "" };
                    details.push_str(mark);
                    details.push_str(text);
                    details.push_str(mark);
                }
            }
            _ => {}
        }
    }
    format!("{title} - {}", strip_escapes(&details))
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
pub(crate) fn missing_annotations(index: &Index, path: &Path) -> Vec<Problem> {
    let Some(module) = index.module(path) else {
        return Vec::new();
    };
    // A module that doesn't parse has its syntax error to show, and its
    // recovered tree's types are no one's.
    if !module.parses {
        return Vec::new();
    }
    let (Some(canonical), Some((Ok(inferred), _))) =
        (&module.canonical, module.types(index.compat))
    else {
        return Vec::new();
    };
    let localizer = localizer(index, module);
    let mut problems = Vec::new();
    for name in module.hir.value_names() {
        if canonical.annotations.contains_key(&name.text) {
            continue;
        }
        let Some(annotation) = inferred.get(&name.text) else {
            continue;
        };
        let ty = render::can_to_doc(&localizer, Context::None, &annotation.ty).plain();
        let message = format!("Missing type annotation: `{ty}`.");
        problems.push(Problem::new(
            path.to_owned(),
            name.range,
            Code::MissingTypeAnnotation,
            message,
        ));
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

    #[test]
    fn codes_read_as_they_are_written() {
        for code in [Code::MissingValue, Code::ElmReview, Code::NoUncurriedPrefix] {
            assert_eq!(Code::parse(code.as_str()), Some(code));
        }
        assert_eq!(Code::parse("elm-review"), None);
    }
}
