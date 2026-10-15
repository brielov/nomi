//! `nomi build`: each case under `nomi_build/cases/` must
//! reproduce the transcript of the same name under `expected/`. nomi's
//! own messages have no oracle, so the transcripts are snapshots, read
//! when written: `NOMI_BLESS=1` writes them anew.
//!
//! Cases run offline, with a cache serving the packages vendored for the
//! compiler's tests.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nomi_testkit::transcript::check_transcripts;
use website::offline;

#[test]
fn nomi_build_transcripts() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_build");
    let cache = website::fixture_cache();
    let bless = std::env::var_os("NOMI_BLESS").is_some();
    check_transcripts(&dir.join("cases"), &dir.join("expected"), |name, case| {
        let work = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(work.path()).unwrap();
        case.create_files(&root).unwrap();
        let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let actual = case
            .run(offline(program, cache.path()), &root)
            .unwrap()
            .compared(case, &root);
        if bless {
            fs::write(dir.join("expected").join(format!("{name}.txt")), &actual).unwrap();
        }
        Some(actual)
    });
}

/// A project built once keeps its dependencies as solved until its
/// elm.json changes, as elm keeps `elm-stuff/d.dat`: the next build
/// reads no package list.
#[test]
fn a_built_project_keeps_its_dependencies_solved() {
    let cache = website::fixture_cache();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let outline = |core: &str| {
        format!(
            r#"{{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {{"direct": {{"elm/core": "{core}", "elm/json": "1.1.4"}}, "indirect": {{}}}}, "test-dependencies": {{"direct": {{}}, "indirect": {{}}}}}}"#
        )
    };
    fs::write(root.join("elm.json"), outline("1.0.5")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("src/Main.elm"),
        "module Main exposing (value)\n\nvalue =\n    1\n",
    )
    .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let build = || {
        let mut command = offline(program, cache.path());
        command.current_dir(&root).args(["check", "src/Main.elm"]);
        command.output().unwrap()
    };
    let first = build();
    assert!(first.status.success(), "{first:?}");
    let index = cache.path().join("registry/index");
    fs::remove_file(&index).unwrap();
    let second = build();
    assert!(second.status.success(), "{second:?}");
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&second.stdout)
    );
    // Changed, it is solved again, and without the package list that fails.
    fs::write(root.join("elm.json"), outline("1.0.4")).unwrap();
    let third = build();
    let said = String::from_utf8_lossy(&third.stdout) + String::from_utf8_lossy(&third.stderr);
    assert!(said.contains("PACKAGE LIST"), "{said}");
}

/// A project in a new directory, its root canonical, whose `Main` is
/// `main`.
fn project(main: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), main).unwrap();
    (work, root)
}

/// `nomi build src/Main.elm` with `args` in `root`, which must succeed.
fn build(cache: &Path, root: &Path, args: &[&str]) {
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let mut build = offline(program, cache);
    build
        .current_dir(root)
        .args(["build", "src/Main.elm"])
        .args(args);
    let built = build.output().unwrap();
    assert!(built.status.success(), "{built:?}");
}

/// What `node script` in `root` writes to standard output, once it
/// succeeds.
fn node(root: &Path, script: &str) -> String {
    let ran = Command::new("node")
        .arg(script)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(ran.status.success(), "{ran:?}");
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

/// A worker built into an `.mjs` file is an ES module exporting `Elm`:
/// Node imports it, and a message sent to its incoming port comes back
/// on its outgoing one.
#[test]
fn a_module_output_is_imported_by_node_and_answers_on_its_ports() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "port module Main exposing (main)


port requests : (String -> msg) -> Sub msg


port responses : String -> Cmd msg


main : Program () () String
main =
    Platform.worker
        { init = \\_ -> ( (), Cmd.none )
        , update = \\request model -> ( model, responses (String.toUpper request) )
        , subscriptions = \\_ -> requests identity
        }
",
    );
    build(cache.path(), &root, &["--output=dist/main.mjs"]);
    let driver = "import { Elm } from './dist/main.mjs';
const app = Elm.Main.init();
app.ports.responses.subscribe((response) => console.log(response));
app.ports.requests.send('ping');
";
    fs::write(root.join("driver.mjs"), driver).unwrap();
    assert_eq!(node(&root, "driver.mjs"), "PING\n");
}

/// A worker built for Node into an `.mjs` file starts when run; built
/// for the browser, the module runs nothing.
#[test]
fn a_node_entry_starts_its_main_and_a_plain_module_runs_nothing() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "module Main exposing (main)


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( Debug.log \"started\" (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/entry.mjs"],
    );
    assert_eq!(node(&root, "dist/entry.mjs"), "started: ()\n");
    build(cache.path(), &root, &["--output=dist/plain.mjs"]);
    assert_eq!(node(&root, "dist/plain.mjs"), "");
}

/// Every file under `dir`, by its path from `dir`, with its contents.
fn files(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut dirs = vec![dir.to_owned()];
    while let Some(at) = dirs.pop() {
        for entry in fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap().to_owned();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// A project built for the browser, then for Node, finds everything it
/// built already: nothing in its artifacts or nomi's cache names a
/// platform.
#[test]
fn building_for_another_platform_writes_no_new_artifact() {
    let cache = website::fixture_cache();
    let (_work, root) = project(
        "module Main exposing (main)

import Page


main : Program () String ()
main =
    Platform.worker
        { init = \\_ -> ( Page.title, Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/html": "1.0.1", "elm/json": "1.1.4"}, "indirect": {"elm/virtual-dom": "1.0.5"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("src/Page.elm"),
        "module Page exposing (title, view)\n\nimport Html exposing (Html)\n\n\ntitle : String\ntitle =\n    \"Home\"\n\n\nview : Html msg\nview =\n    Html.div [] [ Html.text title ]\n",
    )
    .unwrap();
    let artifacts = || {
        let mut project = files(&root);
        project.retain(|path, _| !path.starts_with("dist"));
        (files(cache.path()), project)
    };
    build(cache.path(), &root, &["--output=dist/browser.mjs"]);
    let browser = artifacts();
    build(
        cache.path(),
        &root,
        &["--platform=node", "--output=dist/node.mjs"],
    );
    assert!(browser.0.keys().any(|path| path.starts_with("artifacts")));
    assert!(
        browser == artifacts(),
        "a build for Node changed the artifacts"
    );
}
