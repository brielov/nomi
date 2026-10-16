//! A workspace resolves its members' dependencies together, its member
//! packages taken from the workspace; upstream's commands, run in a
//! member, know nothing of that, nor of what nomi solved. nomi's commands
//! work on the members `-p` names, or every member at the root: each case
//! under `nomi_workspace/cases/`, run in `nomi_workspace/fixture.case`'s
//! workspace, must reproduce the transcript of the same name under
//! `expected/`, which `NOMI_BLESS=1` writes anew.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use nomi_testkit::packages;
use nomi_testkit::transcript::{Case, check_transcripts, normalize_elm_test};
use serde_json::json;
use website::offline;

#[test]
fn nomi_workspace_transcripts() {
    nomi_testkit::require_node();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_workspace");
    let fixture = Case::parse(&fs::read_to_string(dir.join("fixture.case")).unwrap()).unwrap();
    let cache = website::fixture_cache();
    let bless = std::env::var_os("NOMI_BLESS").is_some();
    let ran = check_transcripts(&dir.join("cases"), &dir.join("expected"), |name, case| {
        let (_work, root) = temp();
        fixture.create_files(&root).unwrap();
        case.create_files(&root).unwrap();
        let nomi = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let mut transcript = case.run(offline(nomi, cache.path()), &root).unwrap();
        // What builds leave, and the fixture's files as it wrote them.
        transcript.files.retain(|(path, bytes)| {
            !path.contains("elm-stuff/")
                && !(fixture.files.iter()).any(|(p, text)| p == path && text.as_bytes() == bytes)
        });
        let home = cache.path().display().to_string();
        let shown = root.display().to_string();
        transcript.stdout = normalize_elm_test(&transcript.stdout, &shown, &home);
        transcript.stderr = normalize_elm_test(&transcript.stderr, &shown, &home);
        let actual = transcript.compared(case, &root);
        if bless {
            fs::write(dir.join("expected").join(format!("{name}.txt")), &actual).unwrap();
        }
        Some(actual)
    });
    assert_eq!(ran, 12);
}

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
        "exposed-modules": ["Shared", "Shared.Other"],
        "elm-version": "0.19.0 <= v < 0.20.0",
        "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"},
        "test-dependencies": {},
    })
    .to_string()
}

const MAIN: &str = "module Main exposing (main)\n\nmain =\n    1\n";

/// acme/shared's modules: `Shared` imports `Shared.Base`, and
/// `Shared.Other` imports neither.
const SHARED: [(&str, &str); 3] = [
    (
        "Shared",
        "module Shared exposing (greeting)\n\nimport Shared.Base\n\n\ngreeting : String\n\
         greeting =\n    Shared.Base.hello ++ \"!\"\n",
    ),
    (
        "Shared/Base",
        "module Shared.Base exposing (hello)\n\n\nhello : String\nhello =\n    \"hello\"\n",
    ),
    (
        "Shared/Other",
        "module Shared.Other exposing (other)\n\n\nother : Int\nother =\n    1\n",
    ),
];

/// A worker using both of acme/shared's exposed modules.
const WORKER: &str = "module Main exposing (main)\n\nimport Shared\nimport Shared.Other\n\n\n\
    main : Program () Int ()\nmain =\n    Platform.worker\n        \
    { init = \\() -> ( String.length Shared.greeting + Shared.Other.other, Cmd.none )\n        \
    , update = \\() n -> ( n, Cmd.none )\n        \
    , subscriptions = \\_ -> Sub.none\n        }\n";

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
    for (module, text) in SHARED {
        write(&root.join("packages/shared/src"), module, text);
    }
    (work, root)
}

/// The module `name`, by its path without `.elm`, in `src`.
fn write(src: &Path, name: &str, text: &str) {
    let path = src.join(format!("{name}.elm"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
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
    upstream_unmoved_by_nomi(&root, &root.join("apps/web"), "BAD MAIN TYPE");
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
    assert!(said(&checked).contains("BAD MAIN TYPE"), "{checked:?}");
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

/// How many modules a build with `NOMI_TIMINGS` canonicalized: those it
/// checked rather than took from the artifact cache.
fn canonicalized(output: &Output) -> usize {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr
        .lines()
        .find(|l| l.trim_start().starts_with("Canonicalize "));
    line.map_or(0, |line| {
        let count = line.split_whitespace().last().unwrap();
        count.trim_end_matches('×').parse().unwrap()
    })
}

/// A workspace whose application `apps/web` builds [`WORKER`] with
/// acme/shared, and nomi building it there into `main.js`.
struct Worker {
    _work: tempfile::TempDir,
    root: PathBuf,
    cache: tempfile::TempDir,
}

impl Worker {
    fn new() -> Worker {
        let (work, root) = workspace(&[
            ("apps/web", app(&[("acme/shared", "1.0.0")])),
            ("packages/shared", shared()),
        ]);
        write(&root.join("apps/web/src"), "Main", WORKER);
        Worker {
            _work: work,
            root,
            cache: website::fixture_cache(),
        }
    }

    fn build(&self) -> Output {
        let nomi = Path::new(env!("CARGO_BIN_EXE_nomi"));
        let mut command = offline(nomi, self.cache.path());
        command
            .current_dir(self.root.join("apps/web"))
            .args(["build", "src/Main.elm", "--output=main.js"])
            .env("NOMI_BUNDLE", packages::bundle())
            .env("NOMI_TIMINGS", "1");
        command.output().unwrap()
    }

    fn js(&self) -> String {
        fs::read_to_string(self.root.join("apps/web/main.js")).unwrap()
    }

    fn edit(&self, module: &str, text: &str) {
        write(&self.root.join("packages/shared/src"), module, text);
    }
}

#[test]
fn an_app_builds_with_a_member_package_from_its_source() {
    let worker = Worker::new();
    let built = worker.build();
    assert!(built.status.success(), "{}", said(&built));
    let js = worker.js();
    assert!(js.contains("var $acme$shared$Shared$greeting = "), "{js}");
    assert!(
        js.contains("var $acme$shared$Shared$Base$hello = 'hello';"),
        "{js}"
    );
    assert!(
        js.contains("var $acme$shared$Shared$Other$other = 1;"),
        "{js}"
    );
}

#[test]
fn an_edit_to_a_member_package_rebuilds_its_module_and_what_depends_on_it() {
    let worker = Worker::new();
    assert!(worker.build().status.success());
    let unchanged = worker.build();
    assert!(unchanged.status.success(), "{}", said(&unchanged));
    assert_eq!(canonicalized(&unchanged), 0, "{}", said(&unchanged));
    // A body: Shared.Base alone, as its interface stays.
    let base = "module Shared.Base exposing (hello)\n\n\nhello : String\nhello =\n    \"hi\"\n";
    worker.edit("Shared/Base", base);
    let body = worker.build();
    assert!(body.status.success(), "{}", said(&body));
    assert_eq!(canonicalized(&body), 1, "{}", said(&body));
    assert!(
        worker
            .js()
            .contains("var $acme$shared$Shared$Base$hello = 'hi';")
    );
    // Its interface: Shared, which imports it, too; not Shared.Other, nor
    // Main, as Shared's own interface stays.
    let exposing = base.replace("(hello)", "(hello, extra)");
    worker.edit(
        "Shared/Base",
        &format!("{exposing}\n\nextra : Int\nextra =\n    2\n"),
    );
    let interface = worker.build();
    assert!(interface.status.success(), "{}", said(&interface));
    assert_eq!(canonicalized(&interface), 2, "{}", said(&interface));
}

#[test]
fn an_error_in_a_member_package_is_a_module_error_in_its_file() {
    let worker = Worker::new();
    worker.edit(
        "Shared/Base",
        "module Shared.Base exposing (hello)\n\n\nhello : String\nhello =\n    1\n",
    );
    let built = worker.build();
    assert!(!built.status.success(), "{built:?}");
    let said = said(&built);
    assert!(said.contains("Detected problems in 1 module."), "{said}");
    let bar = said.lines().find(|l| l.starts_with("-- TYPE MISMATCH -"));
    assert!(
        bar.is_some_and(|bar| bar.ends_with("- packages/shared/src/Shared/Base.elm")),
        "{said}"
    );
    assert!(!worker.root.join("apps/web/main.js").exists());
}
