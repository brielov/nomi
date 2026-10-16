//! The `elm.json` files of a workspace's members, for tests that lay one
//! out on disk, and `elm make` in each member of one that is.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::transcript::Case;

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

/// `elm make` in each member of the workspace at `root`, a directory
/// holding an `elm.json` outside `elm-stuff`, by its path from the root:
/// an application's on `src/Main.elm` to `main.js`, a package's on its
/// exposed modules, the case's files the member's own.
pub fn make_cases(root: &Path) -> Vec<(String, Case)> {
    let files = crate::files_below(root).unwrap_or_else(|e| panic!("{}: {e}", root.display()));
    let files: Vec<_> = (files.into_iter())
        .filter(|f| !f.split('/').any(|part| part == "elm-stuff"))
        .collect();
    (files.iter())
        .filter_map(|f| f.strip_suffix("/elm.json"))
        .map(|member| {
            let read = |file: &str| {
                let path = root.join(member).join(file);
                fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            };
            let prefix = format!("{member}/");
            let files = (files.iter())
                .filter_map(|f| f.strip_prefix(&prefix))
                .map(|f| (f.to_owned(), read(f)))
                .collect();
            let package = read("elm.json").contains("\"type\": \"package\"");
            let args = if package {
                vec!["make"]
            } else {
                vec!["make", "src/Main.elm", "--output=main.js"]
            };
            let case = Case {
                args: args.into_iter().map(str::to_owned).collect(),
                files,
                ..Case::default()
            };
            (member.to_owned(), case)
        })
        .collect()
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
