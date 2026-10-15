//! Robustness under editing: random edits to an open module,
//! with requests between them, never crash or hang the server, and once
//! it settles its answers are those of a server that opens the final
//! text fresh.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use nomi_base::{ColumnUnit, LineIndex, TextSize};
use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_testkit::lsp::{self, Session, Step, Timing};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use serde_json::{Map, Value, json};

mod common;

use common::{built, real, replay, store};

/// Bits of Elm, whole and broken, an editor might put in.
const FRAGMENTS: &[&str] = &[
    "x",
    " ",
    "\n",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "\"",
    ",",
    ".",
    "|",
    "=",
    "->",
    "\\x -> ",
    "let\n        y = 1\n    in\n    ",
    "case x of\n        _ ->\n            ",
    "List.map",
    "Html.text",
    "\n\nfoo =\n    1\n",
    "--",
    "{-",
    "-}",
    "import Html\n",
    "1.5",
    "'a'",
    "Shapes.Circle.",
    "module",
    " exposing (..)",
    "type alias A = { a : Int }\n",
];

#[derive(Clone, Debug)]
enum Op {
    /// A fragment inserted at a fraction of the text.
    Insert(f64, usize),
    /// Up to a number of bytes deleted from a fraction of the text.
    Delete(f64, usize),
    /// A request at a fraction of the text.
    Ask(f64, usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0.0..1.0f64, 0..FRAGMENTS.len()).prop_map(|(at, f)| Op::Insert(at, f)),
        (0.0..1.0f64, 1..12usize).prop_map(|(at, n)| Op::Delete(at, n)),
        (0.0..1.0f64, 0..ASKED.len()).prop_map(|(at, m)| Op::Ask(at, m)),
    ]
}

/// The requests asked at a position.
const ASKED: &[&str] = &[
    "textDocument/hover",
    "textDocument/completion",
    "textDocument/definition",
    "textDocument/references",
    "textDocument/documentHighlight",
];

/// The offset `at` of the way into `text`, on a character boundary.
fn offset(text: &str, at: f64) -> usize {
    let mut i = ((text.len() as f64) * at) as usize;
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The LSP position of `offset`, in UTF-16 columns.
fn position(text: &str, offset: usize) -> lsp::Position {
    let at = LineIndex::new(text)
        .line_col(text, TextSize::new(offset as u32), ColumnUnit::Utf16)
        .expect("an offset on a character boundary");
    lsp::Position {
        line: at.line,
        character: at.col,
    }
}

const FILE: &str = "src/Main.elm";

/// The requests whose answers a settled server must agree on.
fn final_requests(text: &str) -> Vec<Step> {
    let mut steps = vec![
        step("textDocument/documentSymbol", None, Map::new()),
        step("textDocument/foldingRange", None, Map::new()),
    ];
    for i in 1..6 {
        let at = position(text, offset(text, f64::from(i) / 6.0));
        for method in [
            "textDocument/hover",
            "textDocument/definition",
            "textDocument/references",
        ] {
            steps.push(step(method, Some(lsp::At::Exact(at)), Map::new()));
        }
    }
    steps
}

fn step(method: &str, at: Option<lsp::At>, params: Map<String, Value>) -> Step {
    Step {
        method: lsp::Method::from(method.to_owned()),
        file: Some(FILE.to_owned()),
        at,
        params,
        resolve: false,
        create: BTreeMap::new(),
    }
}

/// A change of the file from `start` to `end` to `text`.
fn change(start: lsp::Position, end: lsp::Position, text: &str) -> Step {
    let range = json!({ "start": start, "end": end });
    let changes = json!({ "contentChanges": [{ "range": range, "text": text }] });
    let Value::Object(params) = changes else {
        unreachable!("an object literal")
    };
    step("textDocument/didChange", None, params)
}

/// The steps that make `ops` on `original`, and the text they leave.
fn edit_steps(original: &str, ops: &[Op]) -> (Vec<Step>, String) {
    let mut text = original.to_owned();
    let mut steps = Vec::new();
    for op in ops {
        match *op {
            Op::Insert(at, f) => {
                let o = offset(&text, at);
                let start = position(&text, o);
                steps.push(change(start, start, FRAGMENTS[f]));
                text.insert_str(o, FRAGMENTS[f]);
            }
            Op::Delete(at, n) => {
                let o = offset(&text, at);
                let mut end = (o + n).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                steps.push(change(position(&text, o), position(&text, end), ""));
                text.replace_range(o..end, "");
            }
            Op::Ask(at, m) => {
                let at = lsp::At::Exact(position(&text, offset(&text, at)));
                steps.push(step(ASKED[m], Some(at), Map::new()));
            }
        }
    }
    (steps, text)
}

/// Runs `session` in a fresh copy of `case`, its module holding `text`
/// where given.
fn run(
    case: &lsp::Case,
    session: &Session,
    text: Option<&str>,
    scratch: &Path,
    store: &Store,
    memo: &Memo,
    name: &str,
) -> Value {
    let work = scratch.join(name);
    lsp::copy_project(case, &work).unwrap();
    let work = real(&work);
    if let Some(text) = text {
        fs::write(work.join(FILE), text).unwrap();
    }
    let timing = Timing {
        quiet: Duration::from_millis(40),
        answer: Duration::from_secs(20),
        publishes: true,
    };
    replay(&work, session, timing, store, memo, &[])
}

#[test]
fn edits_leave_the_server_as_a_fresh_one() {
    let temp = tempfile::tempdir().unwrap();
    let (store, _) = store(&temp.path().join("store"));
    let memo = built(temp.path(), &store);
    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_language_server/cases");
    let case = (lsp::cases(&cases).into_iter())
        .find(|c| c.name == "structure")
        .expect("the structure case");
    let original = fs::read_to_string(case.dir.join(FILE)).unwrap();
    let mut runner = TestRunner::new(Config {
        cases: std::env::var("NOMI_EDIT_CASES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(12),
        failure_persistence: None,
        ..Config::default()
    });
    let runs = AtomicUsize::new(0);
    runner
        .run(&proptest::collection::vec(op(), 1..24), |ops| {
            let run_number = runs.fetch_add(1, Ordering::Relaxed);
            let (mut steps, text) = edit_steps(&original, &ops);
            steps.push(step("textDocument/didSave", None, Map::new()));
            let finals = final_requests(&text);
            steps.extend(finals.iter().cloned());
            let session = |requests| Session {
                open: vec![FILE.to_owned()],
                requests,
                ..Session::default()
            };
            let played = |session, text, name: String| {
                run(&case, &session, text, temp.path(), &store, &memo, &name)
            };
            let edited = played(session(steps), None, format!("edited-{run_number}"));
            // A server that opens the final text, written to disk.
            let fresh = played(
                session(finals.clone()),
                Some(&text),
                format!("fresh-{run_number}"),
            );
            let answers = edited["requests"].as_array().unwrap();
            let settled = &answers[answers.len() - finals.len()..];
            let saved = &answers[answers.len() - finals.len() - 1];
            prop_assert_eq!(
                &saved["diagnostics"][FILE],
                &fresh["diagnostics"][FILE],
                "diagnostics after\n{}",
                text
            );
            for (a, b) in settled.iter().zip(fresh["requests"].as_array().unwrap()) {
                prop_assert_eq!(a, b, "after\n{}", text);
            }
            Ok(())
        })
        .unwrap();
}
