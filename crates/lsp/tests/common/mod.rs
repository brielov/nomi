//! What the language-server tests share: the fixture packages, in a
//! store as nomi's cache keeps them, sessions replayed against `nomi lsp`
//! on an in-memory connection, and paths named as the server names them.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use lsp_server::Connection;
use nomi_engine::index::{self, Memo};
use nomi_engine::{Store, Vfs};
use nomi_testkit::lsp::{self, Driver, Session, Timing};
use nomi_testkit::packages;
use serde_json::Value;

/// A store holding the fixture packages, laid out as nomi's cache keeps
/// downloads, and what to write for each package's directory in it.
pub(crate) fn store(dir: &Path) -> (Store, Vec<(String, String)>) {
    let mut replacements = Vec::new();
    for package in packages::store(dir).unwrap() {
        let sources = package.sources(dir);
        let to = format!("<packages>/{}/{}", package.name, package.version);
        // As resolved and as the store names it, which differ behind a
        // link (macOS's temporary directory).
        replacements.extend(lsp::dir_replacements(&[&real(&sources), &sources], &to));
    }
    (Store::new(dir.to_owned()), replacements)
}

/// The fixture packages built once, for every server to start from: each
/// would build them again otherwise, which is most of a session's work.
pub(crate) fn built(temp: &Path, store: &Store) -> Memo {
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

/// `session`, replayed against `nomi lsp` with the packages of `store`
/// and `memo` in the project at `work`: its transcript, tidied with
/// `replacements` made and the project's paths written `<project>`.
pub(crate) fn replay(
    work: &Path,
    session: &Session,
    timing: Timing,
    store: &Store,
    memo: &Memo,
    replacements: &[(String, String)],
) -> Value {
    let (server, client) = Connection::memory();
    let (store, memo) = (store.clone(), memo.clone());
    let handle = thread::spawn(move || nomi_lsp::serve(&server, store, memo));
    let transcript = Driver::start(client, work, session, timing).run(session);
    handle.join().unwrap().unwrap();
    let mut replacements = replacements.to_vec();
    replacements.extend(lsp::dir_replacements(&[work], "<project>"));
    lsp::tidy(transcript, &replacements)
}

/// `path` as the server names it: resolved, and on Windows long-named
/// and without the `\\?\` prefix.
pub(crate) fn real(path: &Path) -> PathBuf {
    nomi_engine::canonical(path).unwrap()
}
