//! nomi's own packages, `nomi/*`, come with nomi: a build reads them from
//! its bundle, offline and with none in the store, and `nomi add` adds the
//! release bundled, whatever the registry lists. `NOMI_BUNDLE` bundles a
//! fixture, nomi/fixture 1.0.0, in place of nomi's packages.
#![cfg(unix)]

#[path = "support/server.rs"]
pub mod server;
#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use nomi_base::Compat;
use nomi_registry::{Http, HttpError, Index};
use nomi_testkit::packages;
use server::{Method, Server};
use website::{Website, offline};

/// The packages the fixture registry lists: every fixture release, and
/// nomi/fixture at a release newer than the one bundled.
fn listing() -> Vec<u8> {
    let all = Website::with_sources(&packages::dir())
        .post("https://package.elm-lang.org/all-packages")
        .unwrap();
    let mut all: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&all).unwrap();
    all.insert(
        "nomi/fixture".to_owned(),
        serde_json::json!(["1.0.0", "2.0.0"]),
    );
    serde_json::to_vec(&all).unwrap()
}

/// A website answering `/all-packages` with [`listing`] alone.
struct Listing;

impl Http for Listing {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        panic!("an unexpected GET {url}")
    }

    fn post(&self, _: &str) -> Result<Vec<u8>, HttpError> {
        Ok(listing())
    }
}

/// A cache holding the fixture releases, whose index also lists
/// nomi/fixture 2.0.0.
fn cache() -> tempfile::TempDir {
    let cache = website::fixture_cache();
    let index = Index::fetch(&Listing, Compat::LATEST).unwrap();
    index.write(&cache.path().join("registry/index")).unwrap();
    cache
}

/// An application of `main` depending on elm/core and elm/json, and on
/// `more`.
fn project(more: &str, main: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        format!(
            r#"{{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {{"direct": {{"elm/core": "1.0.5", "elm/json": "1.1.4"{more}}}, "indirect": {{}}}}, "test-dependencies": {{"direct": {{}}, "indirect": {{}}}}}}"#
        ),
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), main).unwrap();
    (work, root)
}

/// `program` with `args` in `root`, its cache `cache` and the fixture
/// bundle, offline, `stdin` its input.
fn run(program: &Path, cache: &Path, root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut command = offline(program, cache);
    command
        .current_dir(root)
        .args(args)
        .env("NOMI_BUNDLE", packages::bundle());
    with_input(command, stdin)
}

fn with_input(mut command: Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn an_app_builds_offline_with_a_bundled_package_and_calls_its_kernel() {
    nomi_testkit::require_bun();
    let cache = cache();
    let (_work, root) = project(
        r#", "nomi/fixture": "1.0.0""#,
        "module Main exposing (main)

import Nomi.Fixture


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( Debug.log (Nomi.Fixture.shout \"hello\") (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
",
    );
    // An entry, which starts its `main` when run.
    fs::write(
        root.join("nomi.toml"),
        "[[entry]]\nmain = \"src/Main.elm\"\nplatform = \"bun\"\noutput = \"dist/main.mjs\"\n",
    )
    .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let built = run(program, cache.path(), &root, &["build"], "");
    assert!(built.status.success(), "{built:?}");
    assert!(!cache.path().join("packages/releases/nomi").exists());
    let ran = Command::new(nomi_testkit::bun())
        .arg("dist/main.mjs")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(ran.status.success(), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "HELLO!: ()\n");
}

#[test]
fn nomi_add_writes_the_bundled_release_and_asks_the_registry_nothing_of_it() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let website = Website::with_sources(&packages::dir());
    let all = listing();
    let paths = asked.clone();
    let server = Server::start(Arc::new(move |method: Method, path: &str, _: &[u8]| {
        paths.lock().unwrap().push(path.to_owned());
        match (method, path) {
            (Method::Post, "/all-packages") => Some(all.clone()),
            (Method::Post, _) => Some(b"[]".to_vec()),
            (Method::Get, p) => website
                .get(&format!("https://package.elm-lang.org{p}"))
                .ok(),
        }
    }));
    let cache = website::fixture_cache();
    fs::remove_file(cache.path().join("registry/index")).unwrap();
    let (_work, root) = project("", "module Main exposing (main)\n\nmain =\n    1\n");
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let mut command = offline(program, cache.path());
    command
        .current_dir(&root)
        .args(["add", "nomi/fixture"])
        .env("NOMI_BUNDLE", packages::bundle())
        .env("NOMI_REGISTRY", &server.url)
        .env_remove("ALL_PROXY");
    let added = with_input(command, "y\n");
    assert!(added.status.success(), "{added:?}");
    let outline: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("elm.json")).unwrap()).unwrap();
    assert_eq!(
        outline["dependencies"]["direct"]["nomi/fixture"], "1.0.0",
        "{outline}"
    );
    let asked = asked.lock().unwrap();
    assert!(asked.contains(&"/all-packages".to_owned()), "{asked:?}");
    assert!(!asked.iter().any(|path| path.contains("nomi")), "{asked:?}");
}

/// Upstream's commands know nothing of nomi's bundle: they take nomi's
/// own packages from the registry, as upstream would, when it lists them.
#[test]
fn elm_install_takes_nomis_own_from_the_registry() {
    let registry = tempfile::tempdir().unwrap();
    for author in ["elm", "elm-explorations"] {
        let fixtures = packages::dir().join(author);
        std::os::unix::fs::symlink(fixtures, registry.path().join(author)).unwrap();
    }
    let published = registry.path().join("nomi/fixture/2.0.0");
    fs::create_dir_all(published.join("src/Nomi")).unwrap();
    let elm_json = r#"{"type": "package", "name": "nomi/fixture", "summary": "published", "license": "BSD-3-Clause", "version": "2.0.0", "exposed-modules": ["Nomi.Fixture"], "elm-version": "0.19.0 <= v < 0.20.0", "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"}, "test-dependencies": {}}"#;
    fs::write(published.join("elm.json"), elm_json).unwrap();
    let module = "module Nomi.Fixture exposing (shout)\n\n\nshout : String -> String\nshout =\n    String.toUpper\n";
    fs::write(published.join("src/Nomi/Fixture.elm"), module).unwrap();
    let website = Website::with_sources(registry.path());
    let all = website
        .post("https://package.elm-lang.org/all-packages")
        .unwrap();
    // An `elm.json` alone, as the website serves one before its archive.
    let releases = registry.path().to_owned();
    let server = Server::start(Arc::new(
        move |method: Method, path: &str, _: &[u8]| match (method, path.strip_prefix("/packages/"))
        {
            (Method::Post, _) if path == "/all-packages" => Some(all.clone()),
            (Method::Post, _) => Some(b"[]".to_vec()),
            (Method::Get, Some(release)) if release.ends_with("/elm.json") => {
                fs::read(releases.join(release)).ok()
            }
            (Method::Get, _) => website
                .get(&format!("https://package.elm-lang.org{path}"))
                .ok(),
        },
    ));
    let cache = website::fixture_cache();
    fs::remove_file(cache.path().join("registry/index")).unwrap();
    let (_work, root) = project("", "module Main exposing (main)\n\nmain =\n    1\n");
    let (_bin, elm) = website::personality("elm");
    let mut command = offline(&elm, cache.path());
    command
        .current_dir(&root)
        .args(["install", "nomi/fixture"])
        .env("NOMI_BUNDLE", packages::bundle())
        .env("NOMI_REGISTRY", &server.url)
        .env_remove("ALL_PROXY");
    let installed = with_input(command, "y\n");
    assert!(installed.status.success(), "{installed:?}");
    let outline: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("elm.json")).unwrap()).unwrap();
    assert_eq!(
        outline["dependencies"]["direct"]["nomi/fixture"], "2.0.0",
        "{outline}"
    );
}

/// nomi trusts its own packages' kernel code for coming from its bundle,
/// not for their author: a project of that author anywhere else imports
/// no kernel code, as upstream's rule has it.
#[test]
fn a_package_by_nomis_author_outside_the_bundle_imports_no_kernel_code() {
    let cache = cache();
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let elm_json = r#"{"type": "package", "name": "nomi/foo", "summary": "foo", "license": "BSD-3-Clause", "version": "1.0.0", "exposed-modules": ["Foo"], "elm-version": "0.19.0 <= v < 0.20.0", "dependencies": {"elm/core": "1.0.0 <= v < 2.0.0"}, "test-dependencies": {}}"#;
    fs::write(root.join("elm.json"), elm_json).unwrap();
    fs::create_dir_all(root.join("src/Elm/Kernel")).unwrap();
    let foo = "module Foo exposing (x)\n\nimport Elm.Kernel.Foo\n\n\nx : Int\nx =\n    Elm.Kernel.Foo.x\n";
    fs::write(root.join("src/Foo.elm"), foo).unwrap();
    fs::write(root.join("src/Elm/Kernel/Foo.js"), "var _Foo_x = 1;\n").unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let checked = run(program, cache.path(), &root, &["check"], "");
    let said = String::from_utf8_lossy(&checked.stderr);
    assert!(!checked.status.success(), "{checked:?}");
    assert!(said.contains("-- MODULE NOT FOUND"), "{said}");
}

#[test]
fn a_project_asking_for_a_release_nomi_doesnt_bundle_is_told_which_it_does() {
    let cache = cache();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let main = "module Main exposing (main)\n\nmain : Program () () ()\nmain =\n    Platform.worker { init = \\_ -> ( (), Cmd.none ), update = \\_ m -> ( m, Cmd.none ), subscriptions = \\_ -> Sub.none }\n";
    // nomi's own bundle, whose nomi/core is 2.0.0.
    let (_old, root) = project(r#", "nomi/core": "1.0.0""#, main);
    let made = offline(program, cache.path())
        .current_dir(&root)
        .args(["make", "src/Main.elm", "--output=/dev/null"])
        .output()
        .unwrap();
    assert_eq!(made.status.code(), Some(1), "{made:?}");
    assert_eq!(
        String::from_utf8_lossy(&made.stderr),
        "-- PACKAGE NOT BUNDLED ------------------------------------------------ elm.json

Your elm.json asks for nomi/core 1.0.0, but nomi/core comes with nomi, never
from the package registry, and this nomi bundles nomi/core 2.0.0.

Ask for that one instead:

    \"nomi/core\": \"2.0.0\"

A new major version changes the package's API, so code written for the old one
may need changes too.

"
    );
    // The fixture bundle, whose nomi/fixture is 1.0.0, though the registry
    // lists 2.0.0.
    let (_new, root) = project(r#", "nomi/fixture": "2.0.0""#, main);
    let made = run(
        program,
        cache.path(),
        &root,
        &["make", "src/Main.elm", "--output=/dev/null"],
        "",
    );
    assert_eq!(made.status.code(), Some(1), "{made:?}");
    let said = String::from_utf8_lossy(&made.stderr);
    assert!(
        said.contains("this nomi bundles nomi/fixture 1.0.0."),
        "{said}"
    );
    assert!(
        said.contains("This nomi is older than the release asked for"),
        "{said}"
    );
}
