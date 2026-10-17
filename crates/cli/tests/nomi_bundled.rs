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
    nomi_testkit::require_node();
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
        "[[entry]]\nmain = \"src/Main.elm\"\nplatform = \"node\"\noutput = \"dist/main.mjs\"\n",
    )
    .unwrap();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let built = run(program, cache.path(), &root, &["build"], "");
    assert!(built.status.success(), "{built:?}");
    assert!(!cache.path().join("packages/releases/nomi").exists());
    let ran = Command::new("node")
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

/// Upstream's commands know nothing of nomi's own packages, bundled or
/// listed.
#[test]
fn elm_install_knows_no_bundled_package() {
    let cache = cache();
    let (_work, root) = project("", "module Main exposing (main)\n\nmain =\n    1\n");
    let (_bin, elm) = website::personality("elm");
    let installed = run(
        &elm,
        cache.path(),
        &root,
        &["install", "nomi/fixture"],
        "y\n",
    );
    assert!(!installed.status.success(), "{installed:?}");
    let said =
        String::from_utf8_lossy(&installed.stdout) + String::from_utf8_lossy(&installed.stderr);
    assert!(said.contains("UNKNOWN PACKAGE"), "{said}");
    assert!(
        !fs::read_to_string(root.join("elm.json"))
            .unwrap()
            .contains("nomi/")
    );
}
