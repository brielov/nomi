//! The language server across a workspace's members and a legacy repo's
//! overlapping source directories: each file has one owner, whose
//! problems alone it shows, and an application reaches into the member
//! packages it uses for its diagnostics, hover and definitions.

use std::time::Duration;

use nomi_testkit::lsp::{Session, Timing};
use nomi_testkit::write_file;
use serde_json::{Value, json};

mod common;

use common::{built, real, replay, store};

/// An application of `src` and `more` that depends on the fixture's
/// elm/core and elm/json, and on each of `deps` at 1.0.0.
fn app(more: &[&str], deps: &[&str]) -> String {
    let mut dirs = vec!["src"];
    dirs.extend(more);
    let mut direct = json!({ "elm/core": "1.0.5", "elm/json": "1.1.4" });
    for dep in deps {
        direct[dep] = json!("1.0.0");
    }
    json!({
        "type": "application",
        "source-directories": dirs,
        "elm-version": "0.19.1",
        "dependencies": { "direct": direct, "indirect": {} },
        "test-dependencies": { "direct": {}, "indirect": {} },
    })
    .to_string()
}

const SHARED: &str = r#"{
    "type": "package",
    "name": "acme/shared",
    "summary": "shared",
    "license": "BSD-3-Clause",
    "version": "1.0.0",
    "exposed-modules": ["Shared"],
    "elm-version": "0.19.0 <= v < 0.20.0",
    "dependencies": { "elm/core": "1.0.0 <= v < 2.0.0" },
    "test-dependencies": {}
}
"#;

/// `acme/shared`'s one module, whose `name` is a `kind` of `value`.
fn shared(kind: &str, value: &str) -> String {
    format!("module Shared exposing (name)\n\n\nname : {kind}\nname =\n    {value}\n")
}

/// `Main`, whose `uses` is a `kind`: `from`'s `name`, with a marker on
/// `name`.
fn using(kind: &str, from: &str) -> String {
    format!(
        "module Main exposing (uses)\n\nimport {from}\n\n\nuses : {kind}\nuses =\n    \
         {from}.name\n{pad}--^\n",
        pad = " ".repeat(from.len() + 3),
    )
}

/// `session` replayed against the files `files` lay out: its transcript,
/// paths written `<project>`.
fn transcript(files: &[(&str, String)], session: &Value) -> Value {
    let temp = tempfile::tempdir().unwrap();
    let (store, packages) = store(&temp.path().join("store"));
    let memo = built(temp.path(), &store);
    let work = real(temp.path()).join("work");
    for (path, text) in files {
        write_file(&work, path, text);
    }
    let session: Session = serde_json::from_value(session.clone()).unwrap();
    let timing = Timing {
        quiet: Duration::from_millis(300),
        answer: Duration::from_secs(30),
        publishes: true,
    };
    replay(&work, &session, timing, &store, &memo, &packages)
}

/// The titles of `diagnostics`, as elm's reports begin.
fn titles(diagnostics: &Value) -> Vec<&str> {
    (diagnostics.as_array().unwrap().iter())
        .map(|d| d["message"].as_str().unwrap().split(" - ").next().unwrap())
        .collect()
}

#[test]
fn an_application_reaches_into_the_member_packages_it_uses() {
    let web = "apps/web/src/Main.elm";
    let api = "apps/api/src/Main.elm";
    let pkg = "packages/shared/src/Shared.elm";
    let files = [
        (
            "nomi.toml",
            "[workspace]\nmembers = [\"apps/*\", \"packages/*\"]\n".to_owned(),
        ),
        ("packages/shared/elm.json", SHARED.to_owned()),
        (pkg, shared("String", "\"shared\"")),
        ("apps/web/elm.json", app(&[], &["acme/shared"])),
        (web, using("Int", "Shared")),
        ("apps/api/elm.json", app(&[], &["acme/shared"])),
        (api, using("String", "Shared")),
    ];
    let change = |text: String| {
        json!({ "method": "textDocument/didChange", "file": pkg,
                "params": { "contentChanges": [{ "text": text }] } })
    };
    let session = json!({
        "open": [pkg, web, api],
        "requests": [
            { "method": "textDocument/hover", "file": web, "at": 0 },
            { "method": "textDocument/definition", "file": web, "at": 0 },
            change(shared("String", "1")),
            { "method": "textDocument/didSave", "file": pkg },
            change(shared("Int", "1")),
            { "method": "textDocument/didSave", "file": pkg },
        ],
    });
    let transcript = transcript(&files, &session);
    // web's `Int` is shared's `String`; api agrees with shared.
    let opened = &transcript["diagnostics"];
    assert_eq!(titles(&opened[web]), ["TYPE MISMATCH"], "{opened:#}");
    assert!(
        opened[web][0]["message"]
            .as_str()
            .unwrap()
            .contains("String")
    );
    assert_eq!(titles(&opened[api]), [] as [&str; 0], "{opened:#}");
    assert_eq!(titles(&opened[pkg]), [] as [&str; 0], "{opened:#}");
    let requests = transcript["requests"].as_array().unwrap();
    let hover = requests[0]["result"]["contents"]["value"].as_str().unwrap();
    assert!(hover.contains("name : String"), "{hover}");
    assert_eq!(
        requests[1]["result"],
        json!({
            "uri": format!("<project>/{pkg}"),
            "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 4, "character": 4 } },
        })
    );
    // shared broken: its error is shown once, in its own file, and the
    // applications keep what they had through its last good build.
    let broken = &requests[3]["diagnostics"];
    assert_eq!(titles(&broken[pkg]), ["TYPE MISMATCH"], "{broken:#}");
    assert_eq!(broken[web], opened[web], "{broken:#}");
    assert_eq!(titles(&broken[api]), [] as [&str; 0], "{broken:#}");
    // shared's `name` an `Int`: web agrees with it, and api doesn't.
    let changed = &requests[5]["diagnostics"];
    assert_eq!(titles(&changed[pkg]), [] as [&str; 0], "{changed:#}");
    assert_eq!(titles(&changed[web]), [] as [&str; 0], "{changed:#}");
    assert_eq!(titles(&changed[api]), ["TYPE MISMATCH"], "{changed:#}");
}

#[test]
fn a_directory_two_projects_list_has_one_owner() {
    let a = "a/src/Main.elm";
    let b = "b/src/Main.elm";
    let common = "shared/Common.elm";
    let files = [
        ("a/elm.json", app(&["../shared"], &[])),
        (a, using("Int", "Common")),
        ("b/elm.json", app(&["../shared"], &[])),
        (b, using("Int", "Common")),
        (
            common,
            shared("Int", "\"common\"").replace("Shared", "Common"),
        ),
    ];
    for open in [[a, b, common], [common, b, a]] {
        let transcript = transcript(&files, &json!({ "open": open }));
        let opened = &transcript["diagnostics"];
        // Each project checks Common, but only `a`, first by path, shows
        // its problem.
        assert_eq!(titles(&opened[common]), ["TYPE MISMATCH"], "{opened:#}");
        assert_eq!(titles(&opened[a]), [] as [&str; 0], "{opened:#}");
        assert_eq!(titles(&opened[b]), [] as [&str; 0], "{opened:#}");
    }
}
