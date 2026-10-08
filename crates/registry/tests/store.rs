//! The package store against a fixture registry.

mod support;

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use nomi_base::Compat;
use nomi_project::json::Cursor as JsonCursor;
use nomi_project::{Name, Version};
use nomi_registry::{HttpError, PackageProblem, Store};
use sha1::{Digest, Sha1};
use support::Fixture;

const LATEST: Compat = Compat::LATEST;

const ENDPOINT: &str = "https://package.elm-lang.org/packages/elm/core/1.0.5/endpoint.json";

const ARCHIVE: &str = "https://github.com/elm/core/zipball/1.0.5/";

fn name(s: &str) -> Name {
    Name::parse(s.as_bytes(), JsonCursor::start(LATEST), LATEST).unwrap()
}

fn core() -> (Name, Version) {
    (name("elm/core"), Version::new(1, 0, 5))
}

/// A zip with these entries, in order; names ending in `/` are
/// directories.
fn zip(entries: &[&str]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for entry in entries {
        if entry.ends_with('/') {
            zip.add_directory(*entry, options).unwrap();
        } else {
            zip.start_file(*entry, options).unwrap();
            zip.write_all(format!("contents of {entry}").as_bytes())
                .unwrap();
        }
    }
    zip.finish().unwrap().into_inner()
}

fn sha1(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn endpoint(hash: &str) -> Vec<u8> {
    format!(r#"{{"url":"{ARCHIVE}","hash":"{hash}"}}"#).into_bytes()
}

/// The website, publishing `archive` as elm/core 1.0.5.
fn website(archive: Vec<u8>) -> Fixture {
    Fixture::new(&[])
        .get(ENDPOINT, Ok(endpoint(&sha1(&archive))))
        .get(ARCHIVE, Ok(archive))
}

/// Every file under `dir`, relative and sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap();
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn releases_are_downloaded_once_and_keep_their_sources() {
    let archive = zip(&[
        "elm-core-84f38891/",
        "elm-core-84f38891/elm.json",
        "elm-core-84f38891/LICENSE",
        "elm-core-84f38891/README.md",
        "elm-core-84f38891/docs.json",
        "elm-core-84f38891/src/",
        "elm-core-84f38891/src/Basics.elm",
        "elm-core-84f38891/src/Elm/Kernel/Basics.js",
        "elm-core-84f38891/src/notes.txt",
        "elm-core-84f38891/tests/Main.elm",
    ]);
    let hash = sha1(&archive);
    let http = website(archive);
    let store = tempfile::tempdir().unwrap();
    let store = Store::new(store.path().join("packages"));
    let (core, version) = core();
    assert_eq!(store.get(&core, version).unwrap(), None);
    let dir = store.fetch(&http, &core, version, LATEST).unwrap();
    assert_eq!(dir, store.dir().join("sources").join(&hash));
    assert_eq!(
        files(&dir),
        [
            "LICENSE",
            "README.md",
            "elm.json",
            "src/Basics.elm",
            "src/Elm/Kernel/Basics.js",
            "src/notes.txt",
        ]
    );
    assert_eq!(
        fs::read_to_string(dir.join("src/Basics.elm")).unwrap(),
        "contents of elm-core-84f38891/src/Basics.elm"
    );
    assert_eq!(http.asked(), [ENDPOINT, ARCHIVE]);
    assert_eq!(store.get(&core, version).unwrap(), Some(dir.clone()));
    assert_eq!(store.fetch(&http, &core, version, LATEST).unwrap(), dir);
    assert_eq!(http.asked().len(), 2);
}

#[test]
fn a_damaged_release_is_downloaded_again() {
    let archive = zip(&["root/", "root/elm.json"]);
    let http = website(archive);
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().to_path_buf());
    let (core, version) = core();
    let dir = store.fetch(&http, &core, version, LATEST).unwrap();
    let release = temp.path().join("releases/elm/core/1.0.5");
    for damage in ["", "not a hash\n", &"0".repeat(41)] {
        fs::write(&release, damage).unwrap();
        assert_eq!(store.get(&core, version).unwrap(), None);
        assert_eq!(store.fetch(&http, &core, version, LATEST).unwrap(), dir);
    }
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(store.get(&core, version).unwrap(), None);
    assert_eq!(store.fetch(&http, &core, version, LATEST).unwrap(), dir);
    assert_eq!(files(&dir), ["elm.json"]);
}

#[test]
fn releases_with_the_same_archive_share_its_sources() {
    let archive = zip(&["root/", "root/elm.json"]);
    let hash = sha1(&archive);
    let other = "https://package.elm-lang.org/packages/elm/core/1.0.4/endpoint.json";
    let http = website(archive).get(other, Ok(endpoint(&hash)));
    let store = tempfile::tempdir().unwrap();
    let store = Store::new(store.path().to_path_buf());
    let (core, version) = core();
    let a = store.fetch(&http, &core, version, LATEST).unwrap();
    let b = store
        .fetch(&http, &core, Version::new(1, 0, 4), LATEST)
        .unwrap();
    assert_eq!(a, b);
}

#[test]
fn what_cant_be_downloaded_is_a_package_problem() {
    let (core, version) = core();
    let down = || Err(HttpError::Status(404));
    let archive = zip(&["root/", "root/elm.json"]);
    let fetch = |http: Fixture| {
        let store = tempfile::tempdir().unwrap();
        let problem = Store::new(store.path().to_path_buf())
            .fetch(&http, &core, version, LATEST)
            .unwrap_err();
        assert!(files(store.path()).is_empty(), "{problem:?}");
        problem
    };
    let problem = fetch(Fixture::new(&[]).get(ENDPOINT, down()));
    assert!(
        matches!(&problem, PackageProblem::BadEndpointRequest(url, HttpError::Status(404)) if url == ENDPOINT),
        "{problem:?}"
    );
    for body in [
        &b"{}"[..],
        br#"{"url":"x"}"#,
        br#"{"url":1,"hash":"h"}"#,
        b"<html>",
    ] {
        let problem = fetch(Fixture::new(&[]).get(ENDPOINT, Ok(body.to_vec())));
        assert!(
            matches!(&problem, PackageProblem::BadEndpointContent(url) if url == ENDPOINT),
            "{problem:?}"
        );
    }
    let problem = fetch(
        Fixture::new(&[])
            .get(ENDPOINT, Ok(endpoint(&sha1(&archive))))
            .get(ARCHIVE, down()),
    );
    assert!(
        matches!(&problem, PackageProblem::BadArchiveRequest(url, _) if url == ARCHIVE),
        "{problem:?}"
    );
    // Upstream unzips before it checks the hash.
    let problem = fetch(
        Fixture::new(&[])
            .get(ENDPOINT, Ok(endpoint("0")))
            .get(ARCHIVE, Ok(b"not a zip".to_vec())),
    );
    assert!(
        matches!(&problem, PackageProblem::BadArchiveContent(url) if url == ARCHIVE),
        "{problem:?}"
    );
    let problem = fetch(
        Fixture::new(&[])
            .get(ENDPOINT, Ok(endpoint(&"A".repeat(40))))
            .get(ARCHIVE, Ok(archive.clone())),
    );
    let PackageProblem::BadArchiveHash(url, expected, actual) = problem else {
        panic!("{problem:?}");
    };
    assert_eq!(
        (&url[..], expected, actual),
        (ARCHIVE, "A".repeat(40), sha1(&archive))
    );
}

#[test]
fn entries_that_would_leave_the_release_are_refused() {
    let (core, version) = core();
    for entry in [
        "root/src/../../escaped.elm",
        "root/src/../elm.json",
        "root/src//Main.elm",
        "root/src/./Main.elm",
        "root/src/A\\..\\..\\B.elm",
        "root/src/C:/Main.elm",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("store"));
        let http = website(zip(&["root/", entry]));
        let problem = store.fetch(&http, &core, version, LATEST).unwrap_err();
        assert!(
            matches!(&problem, PackageProblem::BadArchiveContent(url) if url == ARCHIVE),
            "{entry}: {problem:?}"
        );
        assert_eq!(store.get(&core, version).unwrap(), None);
        assert!(files(temp.path()).is_empty(), "{entry}");
    }
}

#[test]
fn an_archive_with_one_entry_has_no_sources() {
    let (core, version) = core();
    let store = tempfile::tempdir().unwrap();
    let store = Store::new(store.path().to_path_buf());
    let http = website(zip(&["root/elm.json"]));
    let dir = store.fetch(&http, &core, version, LATEST).unwrap();
    assert!(files(&dir).is_empty());
}
