//! The full-stack example, `examples/fullstack`, copied with free ports in
//! place of its own and run on the tests' Node binary: from its root it
//! builds every member and its tests pass; under `nomi run --watch` the
//! api answers the web page's origin and both pages are served, and
//! renaming a field in the shared package's codec restarts the api, which
//! then answers with the new name, and reloads the web page, the page
//! that uses no other member left alone.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;

use nomi_testkit::http::{events, request};
use nomi_testkit::process::{Group, Signal, expect, free_port};
use website::offline;

/// The ports a copy of the example uses.
struct Ports {
    api: u16,
    web: u16,
    landing: u16,
}

/// A copy of the example in a new directory, without what building it in
/// place leaves behind, on `ports`.
fn example(ports: &Ports) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/fullstack");
    nomi_testkit::copy_tree(&example, &root, |path| {
        !(path.components()).any(|c| c.as_os_str() == "elm-stuff" || c.as_os_str() == "dist")
    })
    .unwrap();
    let (api, web, landing) = (ports.api, ports.web, ports.landing);
    let api_url = format!("http://localhost:{api}");
    let web_url = format!("http://localhost:{web}");
    replace(
        &root,
        "apps/api/nomi.toml",
        "\"3000\"",
        &format!("\"{api}\""),
        1,
    );
    replace(
        &root,
        "apps/api/nomi.toml",
        "http://localhost:8000",
        &web_url,
        1,
    );
    replace(
        &root,
        "apps/web/nomi.toml",
        "port = 8000",
        &format!("port = {web}"),
        1,
    );
    replace(
        &root,
        "apps/web/public/index.html",
        "http://localhost:3000",
        &api_url,
        1,
    );
    replace(
        &root,
        "apps/landing/nomi.toml",
        "port = 8001",
        &format!("port = {landing}"),
        1,
    );
    (work, root)
}

/// The file at `path` under `root` with its `count` of `from` replaced by
/// `to`, in one write.
fn replace(root: &Path, path: &str, from: &str, to: &str, count: usize) {
    let path = root.join(path);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(
        text.matches(from).count(),
        count,
        "{from:?} in {}",
        path.display()
    );
    fs::write(&path, text.replace(from, to)).unwrap();
}

/// nomi in `root`, offline with `cache`, with `args` and the tests' Node
/// binary's directory first on the PATH.
fn nomi(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let mut nomi = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    nomi.current_dir(root)
        .env("PATH", nomi_testkit::path_with_node())
        .args(args);
    nomi
}

#[test]
fn it_builds_and_its_tests_pass_from_the_root() {
    let cache = website::fixture_cache();
    let ports = Ports {
        api: 3000,
        web: 8000,
        landing: 8001,
    };
    let (_work, root) = example(&ports);
    let built = nomi(cache.path(), &root, &["build"]).output().unwrap();
    let out = String::from_utf8_lossy(&built.stdout);
    assert!(built.status.success(), "{built:?}");
    for output in [
        "apps/api/dist/Main.mjs",
        "apps/web/dist/Main.js",
        "apps/landing/dist/Main.js",
    ] {
        assert!(out.contains(&format!("───> {output}")), "{out}");
        assert!(root.join(output).is_file(), "{output}");
    }
    let tested = nomi(cache.path(), &root, &["test", "--offline", "--no-color"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&tested.stdout);
    assert_eq!(tested.status.code(), Some(0), "{out}{tested:?}");
    let shared = out.find("Testing packages/shared").expect("shared's tests");
    let api = out.find("Testing apps/api").expect("api's tests");
    assert!(
        out[shared..api].contains("Passed:   3\nFailed:   0\n"),
        "{out}"
    );
    assert!(out[api..].contains("Passed:   5\nFailed:   0\n"), "{out}");
}

/// What nomi says on standard error up to its watching again.
fn batch(stderr: &Receiver<String>) -> String {
    expect(stderr, "[nomi] Watching for changes...").join("\n")
}

/// Waits for the api to say it listens on `port`, unless it said so in
/// `said`; what nomi said meanwhile.
fn listening(said: &str, stderr: &Receiver<String>, port: u16) -> String {
    let line = format!("listening on http://localhost:{port}");
    if said.contains(&line) {
        String::new()
    } else {
        expect(stderr, &line).join("\n")
    }
}

#[test]
fn under_watch_an_edit_to_the_shared_codec_restarts_the_api_and_reloads_the_page() {
    nomi_testkit::require_node();
    let cache = website::fixture_cache();
    let ports = Ports {
        api: free_port(),
        web: free_port(),
        landing: free_port(),
    };
    let (_work, root) = example(&ports);
    let mut run = Group::spawn(
        nomi(cache.path(), &root, &["run", "--watch"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    );
    let stderr = run.stderr();
    let started = batch(&stderr);
    for (member, port) in [("apps/web", ports.web), ("apps/landing", ports.landing)] {
        let serving = format!("[nomi] Serving {member} at http://localhost:{port}");
        assert!(started.lines().any(|line| line == serving), "{started}");
    }
    assert!(started.contains("[nomi] Starting apps/api"), "{started}");
    let mut said = vec![listening(&started, &stderr, ports.api), started];

    // The api lets the page's origin read its answers and post to it.
    let origin = format!("http://localhost:{}", ports.web);
    let todos = request(ports.api, "GET", "/todos").unwrap();
    assert_eq!(todos.status, 200, "{todos:?}");
    assert_eq!(
        todos.header("access-control-allow-origin"),
        Some(origin.as_str())
    );
    assert_eq!(
        todos.text(),
        r#"[{"id":1,"title":"Edit packages/shared and watch both apps pick it up","done":false}]"#
    );
    let preflight = request(ports.api, "OPTIONS", "/todos").unwrap();
    assert_eq!(preflight.status, 204, "{preflight:?}");
    assert_eq!(
        preflight.header("access-control-allow-origin"),
        Some(origin.as_str())
    );
    assert_eq!(
        preflight.header("access-control-allow-headers"),
        Some("Content-Type")
    );

    // The page, given the api's address, and its code.
    let page = request(ports.web, "GET", "/").unwrap();
    assert_eq!(page.status, 200, "{page:?}");
    let api_flag = format!("flags: {{ api: \"http://localhost:{}\" }}", ports.api);
    assert!(page.text().contains(&api_flag), "{}", page.text());
    assert!(page.text().contains("/__nomi/events"), "{}", page.text());
    let code = request(ports.web, "GET", "/Main.js").unwrap();
    assert_eq!(code.status, 200);
    assert!(!code.text().contains("completed"));
    // The landing page, which has none of its own, is titled by its
    // member.
    let landing = request(ports.landing, "GET", "/").unwrap();
    assert_eq!(landing.status, 200, "{landing:?}");
    assert!(
        landing.text().contains("<title>landing</title>"),
        "{}",
        landing.text()
    );
    let (status, events) = events(ports.web, "/__nomi/events").unwrap();
    assert_eq!(status, 200);

    // Renaming a field in the shared codec restarts the api and reloads
    // the page, both now using the new name.
    replace(
        &root,
        "packages/shared/src/Todo.elm",
        "\"done\"",
        "\"completed\"",
        2,
    );
    let rebuilt = batch(&stderr);
    assert!(rebuilt.contains("[nomi] Restarting apps/api"), "{rebuilt}");
    assert!(rebuilt.contains("[nomi] Reloading apps/web"), "{rebuilt}");
    assert!(!rebuilt.contains("apps/landing"), "{rebuilt}");
    expect(&events, "event: reload");
    said.push(listening(&rebuilt, &stderr, ports.api));
    said.push(rebuilt);
    let todos = request(ports.api, "GET", "/todos").unwrap();
    assert_eq!(
        todos.text(),
        r#"[{"id":1,"title":"Edit packages/shared and watch both apps pick it up","completed":false}]"#
    );
    let code = request(ports.web, "GET", "/Main.js").unwrap();
    assert!(code.text().contains("completed"));

    run.signal(Signal::TERM);
    assert_eq!(run.ended(), Some(143));
    // Neither the api's start nor its restart repeats DEV mode's advice.
    said.extend(stderr.iter());
    let said = said.join("\n");
    assert!(!said.contains("Compiled in DEV mode"), "{said}");
}
