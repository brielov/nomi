//! Indexing a project as an editor does: each index after the first
//! reuses the modules of the one before whose text is the same and whose
//! project imports' interfaces are as they were.

use std::sync::Arc;

use nomi_engine::check::Checked;
use nomi_engine::index::{self, Index, Memo};
use nomi_engine::{Store, Vfs, live};
use nomi_testkit::write_file;

const ELM_JSON: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": {
        "direct": { "elm/core": "1.0.5" },
        "indirect": { "elm/json": "1.1.4" }
    },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

const MAIN: &str = "module Main exposing (main)\n\nimport A\n\n\nmain : Int\nmain =\n    A.a\n";
const A: &str = "module A exposing (a)\n\nimport B\n\n\na : Int\na =\n    B.b + 1\n";
const B: &str = "module B exposing (b)\n\n\nb : Int\nb =\n    1\n";

/// The names of the modules of `after` that are `before`'s, sorted.
fn reused(before: &Index, after: &Index) -> Vec<String> {
    let mut names: Vec<String> = (after.modules.iter())
        .filter(|m| before.modules.iter().any(|b| Arc::ptr_eq(b, m)))
        .map(|m| m.name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn an_index_reuses_modules_whose_text_and_imports_stayed() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap().join("project");
    write_file(&root, "elm.json", ELM_JSON);
    for (name, text) in [("Main", MAIN), ("A", A), ("B", B)] {
        write_file(&root, &format!("src/{name}.elm"), text);
    }
    let store = dir.path().join("store");
    nomi_testkit::packages::store(&store).unwrap();
    let store = Store::new(store);
    let vfs = Vfs::new();
    let mut memo = Memo::new();
    let main = root.join("src/Main.elm");
    let mut index = |previous: Option<&Index>| {
        index::index_from(&vfs.snapshot(), &store, &main, &mut memo, previous).unwrap()
    };
    let first = index(None);
    let unchanged = index(Some(&first));
    assert_eq!(reused(&first, &unchanged), ["A", "B", "Main"]);
    // A body edit leaves B's interface as it was, so only B is read again.
    let b = root.join("src/B.elm");
    vfs.set_overlay(&b, B.replace("    1", "    2")).unwrap();
    let body = index(Some(&unchanged));
    assert_eq!(reused(&unchanged, &body), ["A", "Main"]);
    // A new value changes B's interface, so A, which imports it, is read
    // again; A's own interface stays, so Main is reused still.
    vfs.set_overlay(
        &b,
        format!("{B}\n\nc : Int\nc =\n    2\n").replace("(b)", "(b, c)"),
    )
    .unwrap();
    let exposed = index(Some(&body));
    assert_eq!(reused(&body, &exposed), ["Main"]);
}

#[cfg(unix)]
#[test]
fn a_link_to_nothing_is_no_module_to_index_or_check() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap().join("project");
    write_file(&root, "elm.json", ELM_JSON);
    for (name, text) in [("Main", MAIN), ("A", A), ("B", B)] {
        write_file(&root, &format!("src/{name}.elm"), text);
    }
    std::os::unix::fs::symlink("Gone.elm", root.join("src/C.elm")).unwrap();
    let store = dir.path().join("store");
    nomi_testkit::packages::store(&store).unwrap();
    let store = Store::new(store);
    let snapshot = Vfs::new().snapshot();
    let main = root.join("src/Main.elm");
    let index = index::index_from(&snapshot, &store, &main, &mut Memo::new(), None).unwrap();
    let mut canonical: Vec<&str> = (index.modules.iter())
        .filter(|m| m.canonical.is_some())
        .map(|m| m.name.as_str())
        .collect();
    canonical.sort_unstable();
    assert_eq!(canonical, ["A", "B", "Main"]);
    let manifest = root.join("elm.json");
    let live = live::check(&snapshot, &store, &manifest, &[], &mut Memo::new(), None);
    // Main's `Int` is no program: the one problem, rather than the link's.
    let checked = live.unwrap().unwrap().checked;
    let titles: Option<Vec<(&str, &str)>> = match &checked {
        Checked::Modules(modules) => Some(
            (modules.iter())
                .flat_map(|m| m.problems.iter().map(|p| (m.name.as_str(), &*p.title)))
                .collect(),
        ),
        Checked::Fine | Checked::Project(_) => None,
    };
    assert_eq!(titles, Some(vec![("Main", "BAD MAIN TYPE")]), "{checked:?}");
}
