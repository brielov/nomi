//! nomi/test's effectful tests, bundled with nomi: `nomi test` runs each
//! one's task on Node beside the pure tests, and every reporter reports
//! how it ended as it reports a pure test. `elm-test` knows nothing of
//! nomi/test, as `elm` knows nothing of nomi's packages.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;
use std::process::Output;

use website::offline;

const ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": {"elm/core": "1.0.5"},
        "indirect": {}
    },
    "test-dependencies": {
        "direct": {"elm-explorations/test": "2.2.1", "nomi/core": "1.0.0", "nomi/test": "1.0.0"},
        "indirect": {"elm/bytes": "1.0.8", "elm/html": "1.0.1", "elm/json": "1.1.4", "elm/random": "1.0.0", "elm/time": "1.0.0", "elm/virtual-dom": "1.0.5"}
    }
}
"#;

/// A pure test beside effectful ones that pass, fail, fail their
/// expectation and die.
const TESTS: &str = r#"module Effects exposing (dies, expectationFails, fails, passes, pure)

import Expect
import Nomi.Fs as Fs
import Nomi.Stdio as Stdio
import Nomi.Test exposing (effect)
import Process
import Task
import Test exposing (Test, test)


passes : Test
passes =
    effect "a file reads back" <|
        \env ->
            Fs.write "a.txt" "hello" env.fs
                |> Task.andThen (\_ -> Fs.read "a.txt" env.fs)
                |> Task.mapError Fs.errorToString
                |> Task.andThen (\text -> Stdio.print text env.stdout |> Task.map (\_ -> text))
                |> Task.map (Expect.equal "hello")


fails : Test
fails =
    effect "the task fails" <|
        \_ -> Process.sleep 10 |> Task.andThen (\_ -> Task.fail "boom")


expectationFails : Test
expectationFails =
    effect "each test has a file system of its own" <|
        \env ->
            Fs.exists "a.txt" env.fs
                |> Task.mapError Fs.errorToString
                |> Task.map (Expect.equal True)


dies : Test
dies =
    effect "the task dies" <|
        \_ -> Task.succeed () |> Task.andThen (\_ -> Debug.todo "dead")


pure : Test
pure =
    test "a pure test" <|
        \() -> Expect.equal 2 (1 + 1)
"#;

/// `program` run with `command`, then `args`, in a new project of
/// [`TESTS`], offline with the fixture packages and nomi's own bundle.
fn run(program: &Path, command: &[&str], args: &[&str]) -> Output {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(root.join("elm.json"), ELM_JSON).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    fs::write(root.join("tests/Effects.elm"), TESTS).unwrap();
    let cache = website::fixture_cache();
    offline(program, cache.path())
        .current_dir(&root)
        .args(command)
        .args(["--offline", "--seed", "1", "--no-color"])
        .args(args)
        .output()
        .unwrap()
}

fn nomi_test(args: &[&str]) -> Output {
    run(Path::new(env!("CARGO_BIN_EXE_nomi")), &["test"], args)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn effectful_tests_run_beside_pure_ones() {
    nomi_testkit::require_node();
    let output = nomi_test(&["--workers", "2"]);
    let (out, err) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(2), "{out}\n{err}");
    assert_eq!(err, "", "{out}");
    let failures = "Running 5 tests. To reproduce these results, run: elm-test --fuzz 100 --seed 1\n\n\
        ↓ Effects\n✗ the task dies\n\n    This test failed because its task died: \"Error: TODO in module `Effects` on line 41\n    \n    dead\"\n\n\n\
        ↓ Effects\n✗ each test has a file system of its own\n\n    ▼▼▼▼ \n    False\n    ╷\n    │ Expect.equal\n    ╵\n    True\n    ▲▲▲ \n\n\n\
        ↓ Effects\n✗ the task fails\n\n    boom\n\n\n\nTEST RUN FAILED\n";
    assert!(out.contains(failures), "{out}");
    assert!(out.contains("Passed:   2\nFailed:   3\n"), "{out}");
    // The test's output was kept, not printed.
    assert!(!out.contains("hello"), "{out}");
}

#[test]
fn the_json_report_has_effectful_tests_results() {
    nomi_testkit::require_node();
    let output = nomi_test(&["--report", "json", "--workers", "1"]);
    let out = text(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{out}");
    let events: Vec<serde_json::Value> = (out.lines())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let completed: Vec<(String, String, Vec<String>)> = (events.iter())
        .filter(|event| event["event"] == "testCompleted")
        .map(|event| {
            let labels = event["labels"].as_array().unwrap();
            let messages = (event["failures"].as_array().unwrap().iter())
                .map(|failure| failure["message"].as_str().unwrap().to_owned())
                .collect();
            (
                labels.last().unwrap().as_str().unwrap().to_owned(),
                event["status"].as_str().unwrap().to_owned(),
                messages,
            )
        })
        .collect();
    let completed: Vec<(&str, &str, Vec<&str>)> = (completed.iter())
        .map(|(label, status, messages)| {
            let messages = messages.iter().map(String::as_str).collect();
            (label.as_str(), status.as_str(), messages)
        })
        .collect();
    assert_eq!(
        completed,
        [
            (
                "the task dies",
                "fail",
                vec![
                    "This test failed because its task died: \"Error: TODO in module `Effects` on line 41\n\ndead\""
                ]
            ),
            (
                "each test has a file system of its own",
                "fail",
                vec!["Expect.equal"]
            ),
            ("the task fails", "fail", vec!["boom"]),
            ("a file reads back", "pass", vec![]),
            ("a pure test", "pass", vec![]),
        ]
    );
    let last = events.last().unwrap();
    assert_eq!(
        (&last["event"], &last["passed"], &last["failed"]),
        (&"runComplete".into(), &"2".into(), &"3".into())
    );
}

#[test]
fn elm_test_knows_nothing_of_nomi_test() {
    nomi_testkit::require_node();
    let (_bin, elm_test) = website::personality("elm-test");
    let output = run(&elm_test, &[], &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        text(&output.stderr).contains("I could not find versions of the project's dependencies"),
        "{output:?}"
    );
}
