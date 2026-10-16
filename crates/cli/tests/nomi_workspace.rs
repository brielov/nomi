//! A workspace resolves its members' dependencies together, its member
//! packages taken from the workspace; upstream's commands, run in a
//! member, know nothing of that, nor of what nomi solved.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use nomi_testkit::packages;
use serde_json::json;
use website::offline;

/// An application's `elm.json`, depending on elm/core and elm/json and
/// on each of `more` at its version.
fn app(more: &[(&str, &str)]) -> String {
    let mut direct = json!({"elm/core": "1.0.5", "elm/json": "1.1.4"});
    for &(name, version) in more {
        direct[name] = json!(version);
    }
    json!({
        "type": "application",
        "source-directories": ["src"],
        "elm-version": "0.19.1",
        "dependencies": {"direct": direct, "indirect": {}},
        "test-dependencies": {"direct": {}, "indirect": {}},
    })
    .to_string()
}

/// The `elm.json` of acme/shared 1.0.0, which no registry has.
fn shared() -> String {
    json!({
        "type": "package",
        "name": "acme/shared",
        "summary": "shared",
        "license": "BSD-3-Clause",
        "version": "1.0.0",
        "exposed-modules": ["Shared"],
        "elm-version": "0.19.0 <= v < 0.20.0",
        "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"},
        "test-dependencies": {},
    })
    .to_string()
}

const MAIN: &str = "module Main exposing (main)\n\nmain =\n    1\n";

/// A workspace of `apps/*` and `packages/*`, each member a directory and
/// its `elm.json`.
fn workspace(members: &[(&str, String)]) -> (tempfile::TempDir, PathBuf) {
    let (work, root) = temp();
    fs::write(
        root.join("nomi.toml"),
        "[workspace]\nmembers = [\"apps/*\", \"packages/*\"]\n",
    )
    .unwrap();
    for (dir, elm_json) in members {
        project(&root.join(dir), elm_json);
    }
    (work, root)
}

/// A new directory, by its canonical path.
fn temp() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    (work, root)
}

/// The project in `dir` of `elm_json`, with a `src/Main.elm`.
fn project(dir: &Path, elm_json: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("elm.json"), elm_json).unwrap();
    fs::write(dir.join("src/Main.elm"), MAIN).unwrap();
}

/// `program` with `args` in `dir`, offline, its cache `cache`, nomi's
/// packages the fixture bundle.
fn run(program: &Path, cache: &Path, dir: &Path, args: &[&str]) -> Output {
    let mut command = offline(program, cache);
    command
        .current_dir(dir)
        .args(args)
        .env("NOMI_BUNDLE", packages::bundle());
    command.output().unwrap()
}

fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

/// `elm make` in `dir` says the same before and after `nomi check`
/// there, which resolves the dependencies upstream can't and keeps the
/// solution; `nomi check` says `nomi_says`.
fn upstream_unmoved_by_nomi(root: &Path, dir: &Path, nomi_says: &str) {
    let cache = website::fixture_cache();
    // A link, as installed: a copy would be another build to the cache,
    // whose keys would never meet nomi's.
    let nomi = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let bin = tempfile::tempdir().unwrap();
    let elm = bin.path().join("elm");
    std::os::unix::fs::symlink(nomi, &elm).unwrap();
    let make = ["make", "src/Main.elm", "--output=/dev/null"];
    let before = run(&elm, cache.path(), dir, &make);
    assert!(!before.status.success(), "{before:?}");
    let checked = run(nomi, cache.path(), dir, &["check", "src/Main.elm"]);
    assert!(said(&checked).contains(nomi_says), "{checked:?}");
    let after = run(&elm, cache.path(), dir, &make);
    assert_eq!(said(&after), said(&before), "in {}", root.display());
    assert_eq!(after.status.code(), before.status.code());
}

#[test]
fn elm_make_in_a_member_ignores_the_workspace_and_what_nomi_solved() {
    let (_work, root) = workspace(&[
        ("apps/web", app(&[("acme/shared", "1.0.0")])),
        ("packages/shared", shared()),
    ]);
    upstream_unmoved_by_nomi(&root, &root.join("apps/web"), "MEMBER PACKAGE");
}

#[test]
fn elm_make_ignores_what_nomi_solved_with_its_own_packages() {
    let (_work, root) = temp();
    project(&root, &app(&[("nomi/fixture", "1.0.0")]));
    upstream_unmoved_by_nomi(&root, &root, "BAD MAIN TYPE");
}

#[test]
fn apps_that_pin_different_releases_are_named() {
    let (_work, root) = workspace(&[
        ("apps/api", app(&[("elm/json", "1.1.3")])),
        ("apps/web", app(&[])),
    ]);
    let cache = website::fixture_cache();
    let nomi = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let checked = run(
        nomi,
        cache.path(),
        &root.join("apps/web"),
        &["check", "src/Main.elm"],
    );
    assert!(!checked.status.success(), "{checked:?}");
    let said = said(&checked);
    let words = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(words.contains("-- MEMBERS DISAGREE --"), "{said}");
    assert!(
        words.contains(
            "apps/api/elm.json pins elm/json 1.1.3, but apps/web/elm.json pins elm/json 1.1.4."
        ),
        "{said}"
    );
}

#[test]
fn a_member_changed_is_resolved_again() {
    let (_work, root) = workspace(&[
        ("apps/web", app(&[("acme/shared", "1.0.0")])),
        ("packages/shared", shared()),
    ]);
    let cache = website::fixture_cache();
    let nomi = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let web = root.join("apps/web");
    let check = ["check", "src/Main.elm"];
    let checked = run(nomi, cache.path(), &web, &check);
    assert!(said(&checked).contains("MEMBER PACKAGE"), "{checked:?}");
    let elm_json = root.join("packages/shared/elm.json");
    let bumped = fs::read_to_string(&elm_json)
        .unwrap()
        .replace("\"1.0.0\"", "\"1.1.0\"");
    fs::write(&elm_json, bumped).unwrap();
    let checked = run(nomi, cache.path(), &web, &check);
    let words = said(&checked)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        words.contains(
            "apps/web/elm.json pins acme/shared 1.0.0, but acme/shared is the workspace member \
             packages/shared at 1.1.0."
        ),
        "{checked:?}"
    );
}
