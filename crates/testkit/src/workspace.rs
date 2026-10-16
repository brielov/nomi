//! The `elm.json` files of a workspace's members, for tests that lay one
//! out on disk.

use serde_json::{Map, Value, json};

/// An application's `elm.json` that depends directly on each of `deps`,
/// and in its tests on each of `test_deps`, at 1.0.0.
pub fn app(deps: &[&str], test_deps: &[&str]) -> String {
    let mut direct = pinned(deps);
    direct.insert("elm/core".to_owned(), json!("1.0.5"));
    direct.insert("elm/json".to_owned(), json!("1.1.3"));
    json!({
        "type": "application",
        "source-directories": ["src"],
        "elm-version": "0.19.1",
        "dependencies": {"direct": direct, "indirect": {}},
        "test-dependencies": {"direct": pinned(test_deps), "indirect": {}},
    })
    .to_string()
}

/// The `elm.json` of the package `name` at 1.0.0, which depends on each
/// of `deps`, and in its tests on each of `test_deps`, from 1.0.0.
pub fn package(name: &str, deps: &[&str], test_deps: &[&str]) -> String {
    let mut deps = ranges(deps);
    deps.insert("elm/core".to_owned(), json!("1.0.0 <= v < 2.0.0"));
    json!({
        "type": "package",
        "name": name,
        "summary": "fixture",
        "license": "BSD-3-Clause",
        "version": "1.0.0",
        "exposed-modules": ["Fixture"],
        "elm-version": "0.19.0 <= v < 0.20.0",
        "dependencies": deps,
        "test-dependencies": ranges(test_deps),
    })
    .to_string()
}

fn pinned(names: &[&str]) -> Map<String, Value> {
    names
        .iter()
        .map(|&n| (n.to_owned(), json!("1.0.0")))
        .collect()
}

fn ranges(names: &[&str]) -> Map<String, Value> {
    (names.iter())
        .map(|&n| (n.to_owned(), json!("1.0.0 <= v < 2.0.0")))
        .collect()
}
