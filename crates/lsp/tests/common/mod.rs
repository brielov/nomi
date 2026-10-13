//! What the language-server tests share: the fixture packages, in a
//! store as nomi's cache keeps them, and paths named as the server names
//! them.

use std::fs;
use std::path::{Path, PathBuf};

use lsp_types::Url;
use nomi_engine::index::{self, Memo};
use nomi_engine::{Store, Vfs};
use nomi_testkit::lsp;

pub(crate) fn packages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages")
}

/// A store holding the fixture packages, laid out as nomi's cache keeps
/// downloads, and what to write for each package's directory in it.
pub(crate) fn store(dir: &Path) -> (Store, Vec<(String, String)>) {
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
        let to = format!("<packages>/{name}/{version}");
        // As resolved and as the store names it, which differ behind a
        // link (macOS's temporary directory); the longer first, since it
        // holds the other.
        for sources in [real(&sources), sources] {
            let uri = Url::from_directory_path(&sources).unwrap();
            replacements.push((uri.as_str().trim_end_matches('/').to_owned(), to.clone()));
            replacements.push((sources.display().to_string(), to.clone()));
        }
    }
    (Store::new(dir.to_owned()), replacements)
}

pub(crate) fn copy_tree(from: &Path, to: &Path) {
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

/// `path` as the server names it: resolved, and on Windows long-named
/// and without the `\\?\` prefix.
pub(crate) fn real(path: &Path) -> PathBuf {
    nomi_engine::canonical(path).unwrap()
}
