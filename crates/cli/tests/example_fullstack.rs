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

use std::ffi::OsString;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nomi_testkit::http::{events, request};
use nomi_testkit::process::{expect, lines};
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
        !path.components().any(|c| c.as_os_str() == "elm-stuff")
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

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// nomi in `root`, offline with `cache`, with `args` and the tests' Node
/// binary's directory first on the PATH.
fn nomi(cache: &Path, root: &Path, args: &[&str]) -> Command {
    let node = nomi_testkit::node();
    let mut path = OsString::from(node.parent().unwrap());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let mut nomi = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache);
    nomi.current_dir(root).env("PATH", path).args(args);
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
        "apps/api/elm-stuff/nomi/run/src/Main.mjs",
        "apps/web/elm-stuff/nomi/run/src/Main.js",
        "apps/landing/elm-stuff/nomi/run/src/Main.js",
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
fn batch(stderr: &Receiver<String>, run: &mut Child) -> String {
    expect(stderr, "[nomi] Watching for changes...", run).join("\n")
}

/// Waits for the api to say it listens on `port`, unless it said so in
/// `said`.
fn listening(said: &str, stderr: &Receiver<String>, run: &mut Child, port: u16) {
    let line = format!("listening on http://localhost:{port}");
    if !said.contains(&line) {
        expect(stderr, &line, run);
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
    let mut run = nomi(cache.path(), &root, &["run", "--watch"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = lines(run.stderr.take().unwrap());
    let started = batch(&stderr, &mut run);
    // Prefixes are padded to one width.
    for (member, port) in [("apps/web", ports.web), ("apps/landing", ports.landing)] {
        let serving = format!("Serving src/Main.elm at http://localhost:{port}");
        assert!(
            (started.lines())
                .any(|line| line.starts_with(&format!("[{member}] ")) && line.ends_with(&serving)),
            "{started}"
        );
    }
    listening(&started, &stderr, &mut run, ports.api);

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
    let landing = request(ports.landing, "GET", "/").unwrap();
    assert_eq!(landing.status, 200, "{landing:?}");
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
    let rebuilt = batch(&stderr, &mut run);
    assert!(rebuilt.contains("[nomi] Restarting apps/api"), "{rebuilt}");
    assert!(rebuilt.contains("[nomi] Reloading apps/web"), "{rebuilt}");
    assert!(!rebuilt.contains("apps/landing"), "{rebuilt}");
    expect(&events, "event: reload", &mut run);
    listening(&rebuilt, &stderr, &mut run, ports.api);
    let todos = request(ports.api, "GET", "/todos").unwrap();
    assert_eq!(
        todos.text(),
        r#"[{"id":1,"title":"Edit packages/shared and watch both apps pick it up","completed":false}]"#
    );
    let code = request(ports.web, "GET", "/Main.js").unwrap();
    assert!(code.text().contains("completed"));

    signal_term(&run);
    assert_eq!(ended(&mut run), Some(143));
}

fn signal_term(child: &Child) {
    let sent = Command::new("kill")
        .args(["-TERM".to_owned(), child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
}

/// The exit code of `run` once it ends, killed after 30 seconds.
fn ended(run: &mut Child) -> Option<i32> {
    let start = Instant::now();
    while run.try_wait().unwrap().is_none() && start.elapsed().as_secs() < 30 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = run.kill();
    run.wait().unwrap().code()
}
