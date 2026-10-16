//! Plain elm in the full-stack example: nomi invoked as `elm`, in a copy
//! of the example, runs `elm make` in each member as `cargo xtask oracle
//! members` recorded upstream's doing, the workspace around it unseen. A
//! member that depends on no other member builds, to upstream's bytes; one
//! that does fails as upstream's does, the package it names being in no
//! registry.
//!
//! Each runs offline, as each release an application's `elm-version`
//! names and a package as the newest, with a cache serving the packages
//! vendored for the compiler's tests, as elm's `ELM_HOME` held them.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::Path;

use nomi_testkit::transcript::{Transcript, matching, recorded, steady};
use nomi_testkit::workspace::make_cases;
use website::offline;

#[test]
fn independent_members_build_with_plain_elm() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let example = dir.join("../../examples/fullstack");
    let expected = dir.join("tests/elm_members/expected");
    let (_bin, elm) = website::personality("elm");
    let cache = website::fixture_cache();
    let cases = make_cases(&example);
    let manifest = |files: &[(String, String)]| {
        let (_, text) = files.iter().find(|(path, _)| path == "elm.json").unwrap();
        text.clone()
    };
    // Each member package's name, quoted as an `elm.json` depending on it
    // quotes it, by its member.
    let packages: Vec<(&str, String)> = (cases.iter())
        .filter_map(|(member, case)| {
            let json: serde_json::Value = serde_json::from_str(&manifest(&case.files)).unwrap();
            Some((member.as_str(), format!("\"{}\"", json["name"].as_str()?)))
        })
        .collect();
    let mut builds = [0, 0];
    let mut failures = Vec::new();
    for (member, case) in &cases {
        let file = expected.join(format!("{}.txt", member.replace('/', "-")));
        let recordings = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{member}: {e}; rerun `cargo xtask oracle members`"));
        let own = manifest(&case.files);
        let independent =
            !(packages.iter()).any(|(other, name)| other != member && own.contains(name.as_str()));
        for &release in case.releases() {
            let id = format!("{member} as {release}");
            let Some(expected) = recorded(&recordings, release) else {
                panic!("{id}: no transcript; rerun `cargo xtask oracle members`");
            };
            let built = Transcript::parse(expected).unwrap().exit == Some(0);
            assert_eq!(built, independent, "{id}: elm built it: {built}");
            builds[usize::from(built)] += 1;
            let case = case.clone().as_release(release);
            let work = tempfile::tempdir().unwrap();
            let dir = case.lay_out_in(&example, member, work.path()).unwrap();
            let actual =
                (case.run(offline(&elm, cache.path()), &dir).unwrap()).compared(&case, &dir);
            if let Err(why) = matching(&steady(expected), &steady(&actual)) {
                failures.push(format!("{id}:\n{why}"));
            }
        }
    }
    assert!(
        builds.iter().all(|&n| n > 0),
        "builds and failures: {builds:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
