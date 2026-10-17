//! Indexing a project as an editor does: each index after the first
//! reuses the modules of the one before whose text is the same and whose
//! project imports' interfaces are as they were.

use std::sync::Arc;

use nomi_engine::check::Checked;
use nomi_engine::index::{self, Index, MemberBuilds, Memo};
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
    let manifest = root.join("elm.json");
    let kept = MemberBuilds::default();
    let mut index = |previous: Option<&Index>| {
        let snapshot = vfs.snapshot();
        index::index_from(
            &snapshot, &store, &manifest, &main, &mut memo, &kept, previous,
        )
        .unwrap()
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
    let index = index::index(&snapshot, &store, &main, &mut Memo::new()).unwrap();
    let mut canonical: Vec<&str> = (index.modules.iter())
        .filter(|m| m.canonical.is_some())
        .map(|m| m.name.as_str())
        .collect();
    canonical.sort_unstable();
    assert_eq!(canonical, ["A", "B", "Main"]);
    let manifest = root.join("elm.json");
    let kept = MemberBuilds::default();
    let live = live::check(
        &snapshot,
        &store,
        &manifest,
        &[],
        &mut Memo::new(),
        &kept,
        None,
    );
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

/// An application's `elm.json` of the source directories `dirs`.
fn app_of(dirs: &[&str]) -> String {
    ELM_JSON.replace(r#"["src"]"#, &format!("{dirs:?}"))
}

#[test]
fn each_file_has_one_owner_whatever_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap();
    let files = [
        // Two projects listing one directory, and one listing a deeper
        // directory of another's.
        ("a/elm.json", app_of(&["src", "../shared"])),
        ("b/elm.json", app_of(&["src", "../shared"])),
        ("c/elm.json", app_of(&["src", "../a/src/Deep"])),
        // A workspace whose application opens alone.
        (
            "ws/nomi.toml",
            "[workspace]\nmembers = [\"web\", \"shared\"]\n".to_owned(),
        ),
        ("ws/web/elm.json", app_of(&["src"])),
        ("ws/shared/elm.json", SHARED_PACKAGE.to_owned()),
    ];
    for (path, text) in &files {
        write_file(&root, path, text);
    }
    let file = |path: &str| root.join(path);
    let manifest = |dir: &str| Some(root.join(dir).join("elm.json"));
    let snapshot = Vfs::new().snapshot();
    let opened = [
        vec![
            file("a/src/Main.elm"),
            file("b/src/Main.elm"),
            file("c/src/Main.elm"),
        ],
        vec![
            file("c/src/Main.elm"),
            file("b/src/Main.elm"),
            file("a/src/Main.elm"),
        ],
    ];
    for open in opened {
        let owners = index::Owners::read(&snapshot, &open);
        let owner = |path: &str| owners.owner(&snapshot, &file(path));
        assert_eq!(owner("shared/Common.elm"), manifest("a"));
        assert_eq!(owner("b/src/Main.elm"), manifest("b"));
        assert_eq!(owner("a/src/Deep/X.elm"), manifest("c"));
        assert_eq!(owner("a/src/X.elm"), manifest("a"));
        assert_eq!(owner("a/tests/X.elm"), manifest("a"));
    }
    // Only b open: the shared directory is b's.
    let owners = index::Owners::read(&snapshot, &[file("b/src/Main.elm")]);
    assert_eq!(
        owners.owner(&snapshot, &file("shared/Common.elm")),
        manifest("b")
    );
    // A member package is its own, though only an application is open.
    let owners = index::Owners::read(&snapshot, &[file("ws/web/src/Main.elm")]);
    let owner = |path: &str| owners.owner(&snapshot, &file(path));
    assert_eq!(owner("ws/shared/src/Shared.elm"), manifest("ws/shared"));
    // A file no project known holds is the nearest elm.json's above it,
    // and none's with none.
    assert_eq!(owner("a/src/Deep/X.elm"), manifest("a"));
    assert_eq!(owner("shared/Common.elm"), None);
}

const SHARED_PACKAGE: &str = r#"{
    "type": "package",
    "name": "acme/shared",
    "summary": "shared",
    "license": "BSD-3-Clause",
    "version": "1.0.0",
    "exposed-modules": ["Shared"],
    "elm-version": "0.19.0 <= v < 0.20.0",
    "dependencies": { "elm/core": "1.0.0 <= v < 2.0.0" },
    "test-dependencies": {}
}
"#;

/// A workspace in `root` of the application `web`, whose `Main` imports
/// `Shared` and `Other` of the member package `shared`, which exposes
/// both; and a store of the fixture packages. Gives back the store.
fn web_and_shared(root: &std::path::Path, store: &std::path::Path) -> Store {
    let web = ELM_JSON.replace(
        r#""elm/core": "1.0.5""#,
        r#""elm/core": "1.0.5", "acme/shared": "1.0.0""#,
    );
    let main = "module Main exposing (value)\n\nimport Other\nimport Shared\n\n\nvalue : Int\nvalue =\n    Shared.name + Other.other\n";
    let files = [
        (
            "nomi.toml",
            "[workspace]\nmembers = [\"web\", \"shared\"]\n".to_owned(),
        ),
        ("web/elm.json", web),
        ("web/src/Main.elm", main.to_owned()),
        (
            "shared/elm.json",
            SHARED_PACKAGE.replace(r#"["Shared"]"#, r#"["Shared", "Other"]"#),
        ),
        (
            "shared/src/Shared.elm",
            "module Shared exposing (name)\n\n\nname : Int\nname =\n    1\n".to_owned(),
        ),
        (
            "shared/src/Other.elm",
            "module Other exposing (other)\n\n\nother : Int\nother =\n    2\n".to_owned(),
        ),
    ];
    for (path, text) in &files {
        write_file(root, path, text);
    }
    nomi_testkit::packages::store(store).unwrap();
    Store::new(store.to_owned())
}

#[test]
fn a_module_a_member_no_longer_exposes_is_read_again_by_its_importers() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap().join("ws");
    let store = web_and_shared(&root, &dir.path().join("store"));
    let vfs = Vfs::new();
    let mut memo = Memo::new();
    let main = root.join("web/src/Main.elm");
    let manifest = root.join("web/elm.json");
    let kept = MemberBuilds::default();
    let mut index = |previous: Option<&Index>| {
        let snapshot = vfs.snapshot();
        index::index_from(
            &snapshot, &store, &manifest, &main, &mut memo, &kept, previous,
        )
        .unwrap()
    };
    let first = index(None);
    assert!(first.named("Main").unwrap().canonical.is_some());
    vfs.set_overlay(&root.join("shared/elm.json"), SHARED_PACKAGE)
        .unwrap();
    let hidden = index(Some(&first));
    // Main imports a module no dependency exposes now.
    assert!(hidden.named("Main").unwrap().canonical.is_none());
}

#[test]
fn a_member_built_from_a_stale_snapshot_is_never_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap().join("ws");
    let store = web_and_shared(&root, &dir.path().join("store"));
    let vfs = Vfs::new();
    let manifest = root.join("web/elm.json");
    let shared = root.join("shared/src/Shared.elm");
    let kept = MemberBuilds::default();
    let check = |snapshot: &nomi_engine::Snapshot| {
        live::check(
            snapshot,
            &store,
            &manifest,
            &[],
            &mut Memo::new(),
            &kept,
            None,
        )
    };
    let stale = vfs.snapshot();
    vfs.set_overlay(
        &shared,
        "module Shared exposing (name)\n\n\nname : Int\nname =\n    3\n",
    )
    .unwrap();
    assert!(
        check(&stale).is_err(),
        "a stale snapshot's check is cancelled"
    );
    // Broken now, the member has no good build to stand for it: the one
    // the stale snapshot began wasn't kept.
    vfs.set_overlay(
        &shared,
        "module Shared exposing (name)\n\n\nname : Int\nname =\n    \"3\"\n",
    )
    .unwrap();
    assert!(check(&vfs.snapshot()).unwrap().is_none());
    vfs.remove_overlay(&shared).unwrap();
    let fixed = check(&vfs.snapshot()).unwrap().unwrap();
    assert!(
        matches!(fixed.checked, Checked::Fine),
        "{:?}",
        fixed.checked
    );
}

#[test]
fn a_member_package_only_tests_use_is_built_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = nomi_engine::canonical(dir.path()).unwrap().join("ws");
    let store = web_and_shared(&root, &dir.path().join("store"));
    let web = ELM_JSON.replace(
        r#""test-dependencies": { "direct": {}"#,
        r#""test-dependencies": { "direct": { "acme/shared": "1.0.0" }"#,
    );
    write_file(&root, "web/elm.json", &web);
    write_file(&root, "web/src/Main.elm", B);
    // The registry's acme/shared, which the member shadows, isn't readable.
    let hash = "0".repeat(40);
    let published = dir.path().join("store");
    write_file(
        &published,
        "releases/acme/shared/1.0.0",
        &format!("{hash}\n"),
    );
    write_file(&published, &format!("sources/{hash}/elm.json"), "{");
    let test = root.join("web/tests/SharedTest.elm");
    write_file(
        &root,
        "web/tests/SharedTest.elm",
        "module SharedTest exposing (one)\n\nimport Shared\n\n\none : Int\none =\n    Shared.name\n",
    );
    let snapshot = Vfs::new().snapshot();
    let index = index::index(&snapshot, &store, &test, &mut Memo::new()).unwrap();
    assert!(index.named("SharedTest").unwrap().canonical.is_some());
}
