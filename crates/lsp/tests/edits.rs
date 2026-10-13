//! Robustness under editing: random edits to an open module,
//! with requests between them, never crash or hang the server, and once
//! it settles its answers are those of a server that opens the final
//! text fresh.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use lsp_server::Connection;
use lsp_types::Url;
use nomi_engine::Store;
use nomi_engine::index::Memo;
use nomi_testkit::lsp::{self, Driver, Session, Step, Timing};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use serde_json::{Value, json};

mod common;

use common::{built, real, store};

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
fn position(text: &str, offset: usize) -> Value {
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let column: usize = before[before.rfind('\n').map_or(0, |i| i + 1)..]
        .chars()
        .map(char::len_utf16)
        .sum();
    json!({ "line": line, "character": column })
}

fn at(text: &str, offset: usize) -> lsp::At {
    serde_json::from_value(position(text, offset)).unwrap()
}

const FILE: &str = "src/Main.elm";

/// The requests whose answers a settled server must agree on.
fn final_requests(text: &str) -> Vec<Step> {
    let mut steps = vec![
        step("textDocument/documentSymbol", None, &json!({})),
        step("textDocument/foldingRange", None, &json!({})),
    ];
    for i in 1..6 {
        let o = offset(text, f64::from(i) / 6.0);
        for method in [
            "textDocument/hover",
            "textDocument/definition",
            "textDocument/references",
        ] {
            steps.push(step(method, Some(at(text, o)), &json!({})));
        }
    }
    steps
}

fn step(method: &str, at: Option<lsp::At>, params: &Value) -> Step {
    serde_json::from_value(json!({
        "method": method,
        "file": FILE,
        "params": params,
    }))
    .map(|mut s: Step| {
        s.at = at;
        s
    })
    .unwrap()
}

/// Runs `session` in a fresh copy of the `structure` case, its module
/// holding `text` where given.
fn run(
    session: &Session,
    text: Option<&str>,
    scratch: &Path,
    store: &Store,
    memo: &Memo,
    name: &str,
) -> Value {
    let case =
        lsp::cases(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_language_server/cases"))
            .into_iter()
            .find(|c| c.name == "structure")
            .expect("the structure case");
    let work = scratch.join(name);
    lsp::copy_project(&case, &work).unwrap();
    let work = real(&work);
    if let Some(text) = text {
        fs::write(work.join(FILE), text).unwrap();
    }
    let root_uri = Url::from_directory_path(&work).unwrap();
    let root_uri = root_uri.as_str().trim_end_matches('/').to_owned();
    let (server, client) = Connection::memory();
    let (store, memo) = (store.clone(), memo.clone());
    let handle = thread::spawn(move || nomi_lsp::serve(&server, store, memo));
    let timing = Timing {
        quiet: Duration::from_millis(40),
        answer: Duration::from_secs(20),
        publishes: true,
    };
    let driver = Driver::start(client, &work, &root_uri, session, timing);
    let transcript = driver.run(session);
    handle.join().unwrap().unwrap();
    lsp::tidy(transcript, &[(root_uri, "<project>".to_owned())])
}

#[test]
fn edits_leave_the_server_as_a_fresh_one() {
    let temp = tempfile::tempdir().unwrap();
    let (store, _) = store(&temp.path().join("store"));
    let memo = built(temp.path(), &store);
    let original = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/elm_language_server/cases/structure")
            .join(FILE),
    )
    .unwrap();
    let mut runner = TestRunner::new(Config {
        cases: std::env::var("NOMI_EDIT_CASES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(12),
        failure_persistence: None,
        ..Config::default()
    });
    let runs = std::sync::atomic::AtomicUsize::new(0);
    runner
        .run(&proptest::collection::vec(op(), 1..24), |ops| {
            let run_number = runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut text = original.clone();
            let mut steps = Vec::new();
            for op in &ops {
                match *op {
                    Op::Insert(at, f) => {
                        let o = offset(&text, at);
                        let start = position(&text, o);
                        let change = json!({ "range": { "start": start, "end": start }, "text": FRAGMENTS[f] });
                        steps.push(step("textDocument/didChange", None, &json!({ "contentChanges": [change] })));
                        text.insert_str(o, FRAGMENTS[f]);
                    }
                    Op::Delete(at, n) => {
                        let o = offset(&text, at);
                        let mut end = (o + n).min(text.len());
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        let range = json!({ "start": position(&text, o), "end": position(&text, end) });
                        steps.push(step("textDocument/didChange", None, &json!({ "contentChanges": [{ "range": range, "text": "" }] })));
                        text.replace_range(o..end, "");
                    }
                    Op::Ask(at_, m) => {
                        let o = offset(&text, at_);
                        steps.push(step(ASKED[m], Some(at(&text, o)), &json!({})));
                    }
                }
            }
            steps.push(step("textDocument/didSave", None, &json!({})));
            let finals = final_requests(&text);
            steps.extend(finals.iter().cloned());
            let edited = Session {
                open: vec![FILE.to_owned()],
                requests: steps,
                ..Session::default()
            };
            let edited = run(&edited, None, temp.path(), &store, &memo, &format!("edited-{run_number}"));
            // A server that opens the final text, written to disk.
            let fresh = Session {
                open: vec![FILE.to_owned()],
                requests: finals.clone(),
                ..Session::default()
            };
            let fresh = run(&fresh, Some(&text), temp.path(), &store, &memo, &format!("fresh-{run_number}"));
            let answers = edited["requests"].as_array().unwrap();
            let settled = &answers[answers.len() - finals.len()..];
            let saved = &answers[answers.len() - finals.len() - 1];
            prop_assert_eq!(&saved["diagnostics"][FILE], &fresh["diagnostics"][FILE], "diagnostics after\n{}", text);
            for (a, b) in settled.iter().zip(fresh["requests"].as_array().unwrap()) {
                prop_assert_eq!(a, b, "after\n{}", text);
            }
            Ok(())
        })
        .unwrap();
}
