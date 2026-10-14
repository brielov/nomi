//! The package index against a fixture registry.

mod support;

use nomi_base::Compat;
use nomi_project::{Name, Version};
use nomi_registry::{Connection, HttpError, Index, Known, RegistryProblem};
use support::Fixture;

const LATEST: Compat = Compat::LATEST;

const ALL_PACKAGES: &str = "https://package.elm-lang.org/all-packages";

const ALL: &str = r#"{"elm/core":["1.0.0","1.0.5","1.0.2"],"elm/json":["1.1.3"]}"#;

/// Where the releases after the first `count` are listed.
fn since(count: usize) -> String {
    format!("{ALL_PACKAGES}/since/{count}")
}

fn name(s: &str) -> Name {
    s.parse().unwrap()
}

fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version::new(major, minor, patch)
}

fn versions(index: &Index, package: &str) -> Vec<Version> {
    index.versions(&name(package)).unwrap().all().collect()
}

#[test]
fn the_whole_index_sorts_each_packages_versions() {
    let http = Fixture::default().answering(ALL_PACKAGES, ALL);
    let index = Index::fetch(&http, LATEST).unwrap();
    assert_eq!(http.asked(), [ALL_PACKAGES]);
    assert_eq!(index.count(), 4);
    assert_eq!(
        index.versions(&name("elm/core")),
        Some(&Known {
            newest: v(1, 0, 5),
            previous: vec![v(1, 0, 2), v(1, 0, 0)],
        })
    );
    assert_eq!(versions(&index, "elm/json"), [v(1, 1, 3)]);
}

#[test]
fn what_isnt_an_index_is_a_data_problem() {
    for body in [
        r#"{"elm/core":[]}"#,
        r#"{"elm/Core":["1.0.0"]}"#,
        r#"{"elm/core":["1.0"]}"#,
        r#"["elm/core@1.0.0"]"#,
        "<html>",
    ] {
        let http = Fixture::default().answering(ALL_PACKAGES, body);
        assert_eq!(
            Index::fetch(&http, LATEST),
            Err(RegistryProblem::Data(
                ALL_PACKAGES.to_owned(),
                body.as_bytes().to_vec()
            )),
            "{body}"
        );
    }
}

#[test]
fn updates_ask_for_what_came_after_and_put_the_first_newest() {
    let news = r#"["elm/json@1.1.4","elm/http@2.0.0","elm/json@1.1.5"]"#;
    let http = Fixture::default()
        .answering(ALL_PACKAGES, ALL)
        .answering(&since(4), news)
        .answering(&since(7), "[]");
    let index = Index::fetch(&http, LATEST).unwrap();
    let mut updated = index.clone();
    assert_eq!(updated.update(&http, LATEST), Ok(3));
    // `foldr` adds the last release first, so the first is the newest,
    // whatever the numbers say.
    assert_eq!(
        versions(&updated, "elm/json"),
        [v(1, 1, 4), v(1, 1, 5), v(1, 1, 3)]
    );
    assert_eq!(versions(&updated, "elm/http"), [v(2, 0, 0)]);
    assert_eq!(updated.count(), 7);
    let before = updated.clone();
    assert_eq!(updated.update(&http, LATEST), Ok(0));
    assert_eq!(updated, before);
    assert_eq!(http.asked()[1..], [since(4), since(7)]);
    // A failed update leaves the index as it was.
    let http = Fixture::default().answering(&since(4), r#"["elm/json"]"#);
    let mut failed = index.clone();
    assert!(matches!(
        failed.update(&http, LATEST),
        Err(RegistryProblem::Data(..))
    ));
    assert_eq!(failed, index);
}

#[test]
fn the_latest_index_is_saved_and_serves_offline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("registry").join("index");
    let down = || HttpError::Transport("down".to_owned());
    // Nothing saved and no website: no index.
    let http = Fixture::default().failing(ALL_PACKAGES, down());
    assert!(Index::latest(&http, &path, LATEST).is_err());
    assert!(!path.exists());
    // Fetched, then saved.
    let http = Fixture::default().answering(ALL_PACKAGES, ALL);
    let (index, connection) = Index::latest(&http, &path, LATEST).unwrap();
    assert_eq!(connection, Connection::Online);
    assert_eq!(Index::read(&path, LATEST).unwrap(), Some(index));
    // Updated, then saved.
    let http = Fixture::default().answering(&since(4), r#"["elm/core@1.0.6"]"#);
    let (updated, connection) = Index::latest(&http, &path, LATEST).unwrap();
    assert_eq!(connection, Connection::Online);
    assert_eq!(updated.count(), 5);
    assert_eq!(Index::read(&path, LATEST).unwrap(), Some(updated.clone()));
    // The website is down: the saved index, offline.
    let http = Fixture::default().failing(&since(5), down());
    assert_eq!(
        Index::latest(&http, &path, LATEST).unwrap(),
        (updated, Connection::Offline)
    );
    assert_eq!(http.asked(), [since(5)]);
}

#[test]
fn a_damaged_index_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index");
    let http = Fixture::default().answering(ALL_PACKAGES, ALL);
    Index::fetch(&http, LATEST).unwrap().write(&path).unwrap();
    let saved = std::fs::read(&path).unwrap();
    for cut in [0, 5, saved.len() - 1] {
        std::fs::write(&path, &saved[..cut]).unwrap();
        assert_eq!(Index::read(&path, LATEST).unwrap(), None, "cut at {cut}");
    }
    // A length past the end of memory, as damage could write.
    let header = saved.split_inclusive(|&b| b == b'\n').next().unwrap();
    let mut huge = header.to_vec();
    huge.extend_from_slice(format!("{}\n", usize::MAX).as_bytes());
    std::fs::write(&path, &huge).unwrap();
    assert_eq!(Index::read(&path, LATEST).unwrap(), None);
}

#[test]
fn each_release_reads_the_saved_responses_its_way() {
    // 0.19.1 and 0.19.2 keep version numbers in 16 bits.
    let http = Fixture::default().answering(ALL_PACKAGES, r#"{"a/b":["65537.0.0"]}"#);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index");
    Index::fetch(&http, LATEST).unwrap().write(&path).unwrap();
    let read = |compat| versions(&Index::read(&path, compat).unwrap().unwrap(), "a/b");
    assert_eq!(read(Compat::V0_19_1), [v(1, 0, 0)]);
    assert_eq!(read(Compat::V0_19_3), [v(65537, 0, 0)]);
}

#[test]
fn missing_packages_come_with_nearby_names() {
    let http = Fixture::default().answering(ALL_PACKAGES, ALL);
    let index = Index::fetch(&http, LATEST).unwrap();
    let nearby = index.versions_or_nearby(&name("elm/jsn")).unwrap_err();
    assert_eq!(nearby.first(), Some(&&name("elm/json")));
}
