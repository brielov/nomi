//! What parsing must keep for every input, however broken: the fuzz
//! target, the property tests and the corpus check these.

use nomi_base::{Compat, TextRange};

use crate::{
    Parse, Source, SyntaxError, Tool, parse_declarations, parse_expr, parse_expressions,
    parse_module, parse_pattern, parse_type, validate,
};

/// A tool of each kind of project, and elm-format. Releases differ only
/// in 0.19.1's wrapping of columns, which the application tool checks.
const TOOLS: [Tool; 4] = [
    Tool::Elm(Source::Application, Compat::V0_19_1),
    Tool::Elm(Source::Package, Compat::LATEST),
    Tool::Elm(Source::Kernel, Compat::LATEST),
    Tool::ElmFormat,
];

/// A parser's entry point.
type Entry = fn(&str) -> Parse;

/// Parses `text` with every entry point and validates the trees for each
/// kind of tool, panicking if a tree loses text or an error falls outside
/// it or out of source order.
pub fn check_invariants(text: &str) {
    let entries: [(&str, Entry); 6] = [
        ("module", parse_module),
        ("declarations", parse_declarations),
        ("expressions", parse_expressions),
        ("expression", parse_expr),
        ("pattern", parse_pattern),
        ("type", parse_type),
    ];
    for (entry, parse) in entries {
        let parse = parse(text);
        assert!(
            parse.syntax().text() == text,
            "the {entry} tree holds the input"
        );
        check_errors(text, parse.errors(), entry, None);
        for tool in TOOLS {
            check_errors(text, &validate(&parse, tool), entry, Some(tool));
        }
    }
}

/// Checks the errors the `entry` tree has, or `tool`'s validation finds.
fn check_errors(text: &str, errors: &[SyntaxError], entry: &str, tool: Option<Tool>) {
    let inside = |range: TextRange| {
        usize::from(range.end()) <= text.len()
            && text.is_char_boundary(range.start().into())
            && text.is_char_boundary(range.end().into())
    };
    for error in errors {
        assert!(
            inside(error.range),
            "{entry}, {tool:?}: {error:?} is outside the text"
        );
    }
    assert!(
        errors.is_sorted_by_key(|e| e.range.start()),
        "{entry}, {tool:?}: errors are in source order"
    );
}
