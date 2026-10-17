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

/// A release the registry published, as the store keeps it: its name,
/// version, and files.
type Published<'p> = (&'p str, &'p str, &'p [(&'p str, String)]);

/// `session` replayed against the files `files` lay out, the store
/// holding `published` beside the fixture packages: its transcript, paths
/// written `<project>`.
fn transcript(files: &[(&str, String)], published: &[Published], session: &Value) -> Value {
    let temp = tempfile::tempdir().unwrap();
    let (store, packages) = store(&temp.path().join("store"));
    for (n, (name, version, files)) in published.iter().enumerate() {
        let hash = format!("{n:040x}");
        write_file(
            store.dir(),
            &format!("releases/{name}/{version}"),
            &format!("{hash}\n"),
        );
        for (path, text) in *files {
            write_file(store.dir(), &format!("sources/{hash}/{path}"), text);
        }
    }
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
    let extra = "apps/api/src/Extra.elm";
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
        (
            extra,
            "module Extra exposing (x)\n\n\nx : Int\nx =\n    0\n".to_owned(),
        ),
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
            // Opening a file publishes every open file's diagnostics, the
            // change still unsaved.
            { "method": "textDocument/didOpen", "file": extra },
            { "method": "textDocument/didSave", "file": pkg },
            change(shared("Int", "1")),
            { "method": "textDocument/didSave", "file": pkg },
        ],
    });
    let transcript = transcript(&files, &[], &session);
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
    // shared broken, unsaved and then saved: its error is shown once, in
    // its own file, and the applications keep what they had through its
    // last good build.
    for step in [3, 4] {
        let broken = &requests[step]["diagnostics"];
        assert_eq!(titles(&broken[pkg]), ["TYPE MISMATCH"], "{broken:#}");
        assert_eq!(broken[web], opened[web], "{broken:#}");
        assert_eq!(titles(&broken[api]), [] as [&str; 0], "{broken:#}");
    }
    // shared's `name` an `Int`: web agrees with it, and api doesn't.
    let changed = &requests[6]["diagnostics"];
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
        let transcript = transcript(&files, &[], &json!({ "open": open }));
        let opened = &transcript["diagnostics"];
        // Each project checks Common, but only `a`, first by path, shows
        // its problem.
        assert_eq!(titles(&opened[common]), ["TYPE MISMATCH"], "{opened:#}");
        assert_eq!(titles(&opened[a]), [] as [&str; 0], "{opened:#}");
        assert_eq!(titles(&opened[b]), [] as [&str; 0], "{opened:#}");
    }
}

/// The `elm.json` of the package `name` 1.0.0, exposing `module`, that
/// depends on elm/core and on each of `deps`, a name and its range.
fn package(name: &str, module: &str, deps: &[(&str, &str)]) -> String {
    let mut dependencies = json!({ "elm/core": "1.0.0 <= v < 2.0.0" });
    for (dep, range) in deps {
        dependencies[dep] = json!(range);
    }
    json!({
        "type": "package",
        "name": name,
        "summary": name,
        "license": "BSD-3-Clause",
        "version": "1.0.0",
        "exposed-modules": [module],
        "elm-version": "0.19.0 <= v < 0.20.0",
        "dependencies": dependencies,
        "test-dependencies": {},
    })
    .to_string()
}

#[test]
fn a_member_package_builds_as_each_project_using_it_resolves() {
    // `acme/lib`'s `Value` is an `Int` at 1.0.0 and a `String` at 2.0.0.
    let lib = |version: &'static str, kind: &str| {
        let files = vec![
            (
                "elm.json",
                package("acme/lib", "Lib", &[]).replace("1.0.0\"", &format!("{version}\"")),
            ),
            (
                "src/Lib.elm",
                format!(
                    "module Lib exposing (Value, value)\n\n\ntype alias Value =\n    {kind}\n\n\nvalue : Value\nvalue =\n    Debug.todo \"value\"\n"
                ),
            ),
        ];
        files
    };
    let (old, new) = (lib("1.0.0", "Int"), lib("2.0.0", "String"));
    let published: [Published; 2] = [("acme/lib", "1.0.0", &old), ("acme/lib", "2.0.0", &new)];
    let a = "lib/a/src/A.elm";
    let b = "lib/b/src/B.elm";
    // The package `b` builds `a` against lib 2.0.0, the newest its range
    // allows; the application pins lib 1.0.0. Each project's check is in
    // the order of its directory, so `app` is checked before `lib/b` and
    // `web` after.
    for dir in ["app", "web"] {
        let main = format!("{dir}/src/Main.elm");
        let files = [
            (
                "nomi.toml",
                format!("[workspace]\nmembers = [\"lib/*\", \"{dir}\"]\n"),
            ),
            (
                "lib/a/elm.json",
                package("acme/a", "A", &[("acme/lib", "1.0.0 <= v < 3.0.0")]),
            ),
            (
                a,
                "module A exposing (value)\n\nimport Lib\n\n\nvalue : Lib.Value\nvalue =\n    Lib.value\n"
                    .to_owned(),
            ),
            (
                "lib/b/elm.json",
                package("acme/b", "B", &[("acme/a", "1.0.0 <= v < 2.0.0")]),
            ),
            (
                b,
                "module B exposing (b)\n\nimport A\n\n\nb : ()\nb =\n    always () A.value\n"
                    .to_owned(),
            ),
            (
                &format!("{dir}/elm.json"),
                app(&[], &["acme/a", "acme/lib"]),
            ),
            (&main, using("Int", "A").replace("name", "value")),
        ];
        let files: Vec<(&str, String)> = files.iter().map(|(p, t)| (&**p, t.clone())).collect();
        for open in [[b, main.as_str()], [main.as_str(), b]] {
            let transcript = transcript(&files, &published, &json!({ "open": open }));
            let opened = &transcript["diagnostics"];
            assert_eq!(titles(&opened[&main]), [] as [&str; 0], "{dir}: {opened:#}");
            assert_eq!(titles(&opened[b]), [] as [&str; 0], "{dir}: {opened:#}");
        }
    }
}
