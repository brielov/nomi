//! The artifact cache trims itself as a build ends: an entry unused for
//! five days goes, a recent one stays, and a second build within the day
//! leaves the cache alone.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use website::offline;

const ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": { "direct": { "elm/core": "1.0.5", "elm/json": "1.1.4" }, "indirect": {} },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

/// An entry file in the cache's layout, last used `days` ago.
fn entry(cache: &Path, name: char, days: u64) -> PathBuf {
    let dir = cache.join("artifacts/v1/ab");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("ab{}", name.to_string().repeat(62)));
    fs::write(&path, b"stale").unwrap();
    let used = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(used)
        .unwrap();
    path
}

fn check(project: &Path, cache: &Path) {
    let output = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache)
        .args(["check", "src/Main.elm"])
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_build_trims_what_went_unused() {
    let cache = website::fixture_cache();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("elm.json"), ELM_JSON).unwrap();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(
        project.path().join("src/Main.elm"),
        "module Main exposing (one)\n\n\none : Int\none =\n    1\n",
    )
    .unwrap();
    let stale = entry(cache.path(), 'a', 6);
    let recent = entry(cache.path(), 'b', 4);
    check(project.path(), cache.path());
    assert!(!stale.exists(), "an entry unused for six days stays");
    assert!(recent.exists(), "an entry used four days ago goes");
    // Within the day, the next build reads no directory.
    let later = entry(cache.path(), 'c', 6);
    check(project.path(), cache.path());
    assert!(later.exists());
}
