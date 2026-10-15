//! The package store against a fixture registry.

mod support;

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;

use nomi_base::Compat;
use nomi_project::{Name, Version};
use nomi_registry::store::sha1;
use nomi_registry::{Bundle, HttpError, PackageProblem, Store};
use nomi_testkit::packages;
use support::Fixture;

const LATEST: Compat = Compat::LATEST;

const ENDPOINT: &str = "https://package.elm-lang.org/packages/elm/core/1.0.5/endpoint.json";

const ARCHIVE: &str = "https://github.com/elm/core/zipball/1.0.5/";

fn core() -> (Name, Version) {
    ("elm/core".parse().unwrap(), Version::new(1, 0, 5))
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

fn endpoint(hash: &str) -> Vec<u8> {
    format!(r#"{{"url":"{ARCHIVE}","hash":"{hash}"}}"#).into_bytes()
}

/// The website, publishing `archive` as elm/core 1.0.5.
fn website(archive: Vec<u8>) -> Fixture {
    Fixture::default()
        .answering(ENDPOINT, endpoint(&sha1(&archive)))
        .answering(ARCHIVE, archive)
}

/// Every file under `dir`, relative and sorted.
fn files(dir: &Path) -> Vec<String> {
    nomi_testkit::files_below(dir).unwrap()
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
    let http = website(archive).answering(other, endpoint(&hash));
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
    let down = || HttpError::Status(404);
    let archive = zip(&["root/", "root/elm.json"]);
    let fetch = |http: Fixture| {
        let store = tempfile::tempdir().unwrap();
        let problem = Store::new(store.path().to_path_buf())
            .fetch(&http, &core, version, LATEST)
            .unwrap_err();
        assert!(files(store.path()).is_empty(), "{problem:?}");
        problem
    };
    let problem = fetch(Fixture::default().failing(ENDPOINT, down()));
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
        let problem = fetch(Fixture::default().answering(ENDPOINT, body));
        assert!(
            matches!(&problem, PackageProblem::BadEndpointContent(url) if url == ENDPOINT),
            "{problem:?}"
        );
    }
    let problem = fetch(
        Fixture::default()
            .answering(ENDPOINT, endpoint(&sha1(&archive)))
            .failing(ARCHIVE, down()),
    );
    assert!(
        matches!(&problem, PackageProblem::BadArchiveRequest(url, _) if url == ARCHIVE),
        "{problem:?}"
    );
    // Upstream unzips before it checks the hash.
    let problem = fetch(
        Fixture::default()
            .answering(ENDPOINT, endpoint("0"))
            .answering(ARCHIVE, "not a zip"),
    );
    assert!(
        matches!(&problem, PackageProblem::BadArchiveContent(url) if url == ARCHIVE),
        "{problem:?}"
    );
    let problem = fetch(
        Fixture::default()
            .answering(ENDPOINT, endpoint(&"A".repeat(40)))
            .answering(ARCHIVE, &archive),
    );
    let PackageProblem::BadArchiveHash {
        url,
        expected,
        actual,
    } = problem
    else {
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

#[test]
fn nomis_own_packages_come_from_the_bundle_alone() {
    let fixture: Name = "nomi/fixture".parse().unwrap();
    let (one, two) = (Version::new(1, 0, 0), Version::new(2, 0, 0));
    let http = Fixture::default();
    let dir = tempfile::tempdir().unwrap();
    let bundle = Bundle::read(&packages::bundle()).unwrap();
    let store = Store::bundled(dir.path().join("packages"), bundle);
    let bundled = packages::bundle().join("fixture");
    assert_eq!(store.get(&fixture, one).unwrap(), Some(bundled.clone()));
    assert_eq!(store.fetch(&http, &fixture, one, LATEST).unwrap(), bundled);
    assert_eq!(store.versions(&fixture).unwrap(), [one]);
    assert_eq!(store.get(&fixture, two).unwrap(), None);
    assert!(matches!(
        store.fetch(&http, &fixture, two, LATEST),
        Err(PackageProblem::NotBundled)
    ));
    assert_eq!(store.downloaded().unwrap()[&fixture].newest, one);
    assert_eq!(store.newest().unwrap(), [(fixture.clone(), bundled)]);
    // Without the bundle, nothing serves them, not even the website.
    let store = Store::new(dir.path().join("packages"));
    assert_eq!(store.get(&fixture, one).unwrap(), None);
    assert!(matches!(
        store.fetch(&http, &fixture, one, LATEST),
        Err(PackageProblem::NotBundled)
    ));
    assert!(http.asked().is_empty());
}
