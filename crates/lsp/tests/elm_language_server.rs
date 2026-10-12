//! Parity with elm-language-server 2.10.0 (plan 9.1): each case's session,
//! recorded from it by `cargo xtask oracle lsp`, replayed against `nomi
//! lsp` on an in-memory connection, and its diagnostics and each answer
//! compared under the ratchet.
//!
//! nomi gets the fixture packages the recording gave elm-language-server,
//! in a store of its own, and its paths to them are written as the
//! recording writes `ELM_HOME`'s.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use lsp_server::Connection;
use lsp_types::Url;
use nomi_engine::index::{self, Memo};
use nomi_engine::{Store, Vfs};
use nomi_testkit::lsp::{self, Driver, Timing};
use nomi_testkit::ratchet::Suite;
use serde_json::Value;

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/elm_language_server")
}

fn packages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// A store holding the fixture packages, laid out as nomi's cache keeps
/// downloads, and what to write for each package's directory in it.
fn store(dir: &Path) -> (Store, Vec<(String, String)>) {
    let manifest = fs::read_to_string(packages_dir().join("manifest.toml")).unwrap();
    let mut packages = Vec::new();
    let (mut name, mut version) = (String::new(), String::new());
    for line in manifest.lines() {
        let value = || line.split('"').nth(1).unwrap_or_default().to_owned();
        if line.starts_with("name = ") {
            name = value();
        } else if line.starts_with("version = ") {
            version = value();
        } else if line.starts_with("sha1 = ") {
            packages.push((name.clone(), version.clone(), value()));
        }
    }
    let mut replacements = Vec::new();
    for (name, version, hash) in packages {
        let release = dir.join("releases").join(&name).join(&version);
        fs::create_dir_all(release.parent().unwrap()).unwrap();
        fs::write(&release, format!("{hash}\n")).unwrap();
        let sources = dir.join("sources").join(&hash);
        copy_tree(&packages_dir().join(&name).join(&version), &sources);
        let sources = real(&sources);
        let to = format!("<packages>/{name}/{version}");
        let uri = Url::from_directory_path(&sources).unwrap();
        replacements.push((uri.as_str().trim_end_matches('/').to_owned(), to.clone()));
        replacements.push((sources.display().to_string(), to));
    }
    (Store::new(dir.to_owned()), replacements)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

/// The fixture packages built once, for every server to start from: each
/// would build them again otherwise, which is most of a session's work.
fn built(temp: &Path, store: &Store) -> Memo {
    let dir = temp.join("prebuild");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("elm.json"), lsp::DEFAULT_ELM_JSON).unwrap();
    let main = dir.join("src/Main.elm");
    fs::write(&main, "module Main exposing (zero)\n\n\nzero =\n    0\n").unwrap();
    let mut memo = Memo::new();
    let snapshot = Vfs::new().snapshot();
    index::index(&snapshot, store, &main, &mut memo).expect("the fixture packages index");
    memo
}

#[test]
fn sessions_answer_as_elm_language_server_does() {
    let suite = Suite::open(suite_dir());
    let temp = tempfile::tempdir().unwrap();
    let (store, packages) = store(&temp.path().join("store"));
    let memo = built(temp.path(), &store);
    let cases = lsp::cases(&suite_dir().join("cases"));
    // A session spends most of its time waiting for its server to go
    // quiet, so many run side by side.
    let next = AtomicUsize::new(0);
    let threads = thread::available_parallelism().map_or(16, |n| 4 * n.get());
    let mut outcomes: Vec<_> = thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut outcomes = Vec::new();
                    while let Some(case) = cases.get(next.fetch_add(1, Ordering::Relaxed)) {
                        outcomes.extend(replay(case, temp.path(), &store, &memo, &packages));
                    }
                    outcomes
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    outcomes.sort_by(|a, b| a.0.cmp(&b.0));
    suite.finish(outcomes);
}

fn replay(
    case: &lsp::Case,
    temp: &Path,
    store: &Store,
    memo: &Memo,
    packages: &[(String, String)],
) -> Vec<(String, Result<(), String>)> {
    let expected = suite_dir()
        .join("expected")
        .join(format!("{}.json", case.name));
    let expected: Value = serde_json::from_str(&fs::read_to_string(&expected).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", expected.display()));
    let cases = temp.join("cases");
    fs::create_dir_all(&cases).unwrap();
    let work = lsp::project_dir(&real(&cases), &case.name);
    lsp::copy_project(case, &work).unwrap();
    let root_uri = Url::from_directory_path(&work).unwrap();
    let root_uri = root_uri.as_str().trim_end_matches('/').to_owned();

    let (server, client) = Connection::memory();
    let (store, memo) = (store.clone(), memo.clone());
    let handle = thread::spawn(move || nomi_lsp::serve(&server, store, memo));
    let timing = Timing {
        quiet: Duration::from_millis(300),
        answer: Duration::from_secs(30),
        publishes: true,
    };
    let driver = Driver::start(client, &work, &root_uri, &case.session, timing);
    let transcript = driver.run(&case.session);
    handle.join().unwrap().unwrap();

    let mut replacements = packages.to_vec();
    replacements.push((root_uri, "<project>".to_owned()));
    replacements.push((work.display().to_string(), "<project>".to_owned()));
    let actual = lsp::tidy(transcript, &replacements);
    // To compare by hand: nomi's transcripts, as the oracle's are written.
    if let Some(dir) = std::env::var_os("NOMI_LSP_DUMP") {
        let path = Path::new(&dir).join(format!("{}.json", case.name));
        fs::write(path, lsp::to_text(&actual)).unwrap();
    }
    lsp::outcomes(&case.name, &expected, &actual)
}

/// `path` with its links resolved, as the server sees it; on Windows as
/// it is, since a resolved path there doesn't make a URL.
fn real(path: &Path) -> PathBuf {
    if cfg!(windows) {
        path.to_owned()
    } else {
        fs::canonicalize(path).unwrap()
    }
}
