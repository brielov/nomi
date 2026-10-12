use std::fs;
use std::path::Path;

use nomi_engine::Vfs;

use super::*;

const APP: &str = r#"{
    "type": "application",
    "source-directories": ["src"],
    "elm-version": "0.19.1",
    "dependencies": { "direct": { "elm/core": "1.0.5", "elm/json": "1.1.4" }, "indirect": {} },
    "test-dependencies": { "direct": {}, "indirect": {} }
}
"#;

/// A project with `module` at `src/Main.elm`, open in an editor.
fn open(module: &str) -> (tempfile::TempDir, Vfs, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    // Found files have their canonical names, which differ where the
    // temporary directory is behind a link, as on macOS.
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::write(root.join("elm.json"), APP).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let path = root.join("src/Main.elm");
    let vfs = Vfs::new();
    vfs.set_overlay(&path, module).unwrap();
    (dir, vfs, path)
}

fn at(text: &str, range: TextRange) -> &str {
    &text[range]
}

#[test]
fn a_syntax_error_is_elms_at_its_region() {
    let text = "module Main exposing (x)\n\n\nx =\n    = 1\n";
    let (_dir, vfs, path) = open(text);
    let found = diagnostics(&vfs.snapshot(), &path).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].title, "MISSING EXPRESSION");
    assert_eq!(
        u32::from(found[0].range.start()),
        text.find("= 1").unwrap() as u32
    );
    assert!(
        found[0].message.starts_with("I am partway through parsing"),
        "{}",
        found[0].message
    );
    let fine = "module Main exposing (x)\n\n\nx =\n    1\n";
    vfs.set_overlay(&path, fine).unwrap();
    assert_eq!(diagnostics(&vfs.snapshot(), &path).unwrap(), []);
}

#[test]
fn the_project_says_which_release_reads_the_file() {
    let (dir, vfs, path) = open("");
    let context = Context::of(&vfs.snapshot(), &path);
    assert_eq!(context.compat, Compat::V0_19_1);
    assert_eq!(context.source, Source::Application);
    let outside = Context::of(&vfs.snapshot(), Path::new("/nowhere/Main.elm"));
    assert_eq!(outside.compat, Compat::LATEST);
    drop(dir);
}

#[test]
fn positions_count_as_the_release_does() {
    let text = "ab\r\ncd😀e\n";
    // 0.19.1 counts from 1, and a carriage return is no column.
    assert_eq!(elm_offset(text, 1, 3, Compat::V0_19_1), TextSize::new(2));
    assert_eq!(elm_offset(text, 2, 4, Compat::V0_19_1), TextSize::new(10));
    assert_eq!(elm_offset(text, 1, 3, Compat::V0_19_3), TextSize::new(10));
    assert_eq!(elm_offset(text, 9, 0, Compat::V0_19_3), TextSize::of(text));
    assert_eq!(elm_offset(text, 0, 99, Compat::V0_19_3), TextSize::new(3));
}

#[test]
fn formatting_edits_only_what_changes() {
    let before = "module Main exposing (x, y)\n\n\nx = 1\n\n\n\n\ny =\n  2\n";
    let (_dir, vfs, path) = open(before);
    let edits = format(&vfs.snapshot(), &path).unwrap().unwrap();
    let mut after = before.to_owned();
    for edit in edits.iter().rev() {
        after.replace_range(std::ops::Range::<usize>::from(edit.range), &edit.text);
    }
    assert_eq!(after, nomi_format::format(before).unwrap());
    // The header and `x`'s name are kept.
    assert!(
        edits.iter().all(|e| u32::from(e.range.start()) > 30),
        "{edits:?}"
    );
    vfs.set_overlay(&path, "x = = 1\n").unwrap();
    assert_eq!(format(&vfs.snapshot(), &path).unwrap(), None);
}

#[test]
fn edits_of_equal_texts_are_none() {
    assert_eq!(edits("a\nb\n", "a\nb\n"), []);
    let changed = edits("a\nbXc\nd\n", "a\nbYc\nd\n");
    assert_eq!(changed.len(), 1);
    assert_eq!(at("a\nbXc\nd\n", changed[0].range), "X");
    assert_eq!(changed[0].text, "Y");
}

#[test]
fn symbols_are_listed_as_elm_language_server_lists_them() {
    let text = "module Main exposing (..)\n\n\n{-| A shape. -}\ntype Shape\n    = Circle Float\n    | Square\n\n\ntype alias Point =\n    { x : Int }\n\n\narea : Shape -> Float\narea shape =\n    let\n        half =\n            2\n    in\n    half\n\n\nport out : Int -> Cmd msg\n";
    let (_dir, vfs, path) = open(text);
    let symbols = document_symbols(&vfs.snapshot(), &path).unwrap();
    let listed: Vec<(&str, SymbolKind, &str)> = symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind, at(text, s.range)))
        .collect();
    assert_eq!(
        listed,
        [
            ("Main", SymbolKind::Module, "module Main exposing (..)"),
            (
                "Shape",
                SymbolKind::Enum,
                "type Shape\n    = Circle Float\n    | Square"
            ),
            ("Circle Float", SymbolKind::EnumMember, "Circle Float"),
            ("Square", SymbolKind::EnumMember, "Square"),
            (
                "Point",
                SymbolKind::Struct,
                "type alias Point =\n    { x : Int }"
            ),
            (
                "area",
                SymbolKind::Function,
                "area shape =\n    let\n        half =\n            2\n    in\n    half"
            ),
            ("half", SymbolKind::Function, "half =\n            2"),
        ]
    );
}

/// A package store holding the compiler's fixture copy of elm/core 1.0.5.
fn store() -> (tempfile::TempDir, nomi_engine::Store) {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let hash = "0".repeat(40);
    let core =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../compiler/tests/packages/elm/core/1.0.5");
    copy(&core, &dir.path().join("sources").join(&hash));
    let release = dir.path().join("releases/elm/core");
    fs::create_dir_all(&release).unwrap();
    fs::write(release.join("1.0.5"), format!("{hash}\n")).unwrap();
    let store = nomi_engine::Store::new(dir.path().to_owned());
    (dir, store)
}

const NAVIGATED: &str = "module Main exposing (main, Shape(..))

import Other exposing (helper)


type Shape
    = Circle Float


main =
    let
        size =
            helper 2
    in
    area (Circle size) + Other.twice size


area shape =
    case shape of
        Circle r ->
            r * r
";

const OTHER: &str = "module Other exposing (helper, twice)


helper x =
    x


twice x =
    x + x
";

/// The NAVIGATED project, with `Other` saved beside `Main`.
fn navigated() -> (
    tempfile::TempDir,
    Vfs,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let (dir, vfs, path) = open(NAVIGATED);
    let other = fs::canonicalize(dir.path()).unwrap().join("src/Other.elm");
    fs::write(&other, OTHER).unwrap();
    (dir, vfs, path, other)
}

/// The offset of the `n`th `needle` in `text`, plus `plus`.
fn offset(text: &str, needle: &str, n: usize) -> TextSize {
    let at = text.match_indices(needle).nth(n).unwrap().0;
    TextSize::new(at as u32 + 1)
}

/// A buffer the editor hasn't saved, in a project it reaches through a
/// link, as a macOS temporary directory is: the index names it by its
/// canonical path, and finds it by either.
#[cfg(unix)]
#[test]
fn an_unsaved_buffer_behind_a_link_is_found() {
    let (dir, _, _, _) = navigated();
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("project");
    std::os::unix::fs::symlink(fs::canonicalize(dir.path()).unwrap(), &link).unwrap();
    let path = link.join("src/Main.elm");
    let vfs = Vfs::new();
    vfs.set_overlay(&path, NAVIGATED).unwrap();
    let (_store_dir, store) = store();
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, &path, &mut Default::default()).unwrap();
    let found = definition(&snapshot, &index, &path, offset(NAVIGATED, "area", 0)).unwrap();
    assert_eq!(
        found.path,
        fs::canonicalize(dir.path()).unwrap().join("src/Main.elm")
    );
}

#[test]
fn definition_finds_locals_top_level_names_and_other_modules() {
    let (_dir, vfs, path, other) = navigated();
    let (_store_dir, store) = store();
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, &path, &mut Default::default()).unwrap();
    let go = |needle, n| definition(&snapshot, &index, &path, offset(NAVIGATED, needle, n));
    // `size` in the body, to its `let`.
    let found = go("size", 1).unwrap();
    assert_eq!(
        (found.path.as_path(), at(NAVIGATED, found.range)),
        (path.as_path(), "size")
    );
    assert_eq!(
        found.range.start(),
        offset(NAVIGATED, "size", 0) - TextSize::new(1)
    );
    // `area`, to its declaration.
    let found = go("area", 0).unwrap();
    assert_eq!(
        found.range.start(),
        offset(NAVIGATED, "area", 1) - TextSize::new(1)
    );
    // `Circle` in a pattern, to its variant.
    let found = go("Circle", 2).unwrap();
    assert_eq!(
        found.range.start(),
        offset(NAVIGATED, "Circle", 0) - TextSize::new(1)
    );
    // `Other.twice`, to the other module: its name and parameters, as
    // elm-language-server answers.
    let found = go("twice", 0).unwrap();
    assert_eq!(found.path, other);
    assert_eq!(at(OTHER, found.range), "twice x");
    assert_eq!(
        found.range.start(),
        offset(OTHER, "twice", 1) - TextSize::new(1)
    );
    // `+`, to the function elm/core's Basics declares it for.
    let found = go("+", 0).unwrap();
    assert!(found.path.ends_with("src/Basics.elm"), "{found:?}");
    let basics = fs::read_to_string(&found.path).unwrap();
    assert_eq!(&basics[found.range], "add");
}

#[test]
fn references_and_highlights_find_every_use() {
    let (_dir, vfs, path, other) = navigated();
    let (_store_dir, store) = store();
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, &path, &mut Default::default()).unwrap();
    // `helper`, from its use in Main: with the declaration, Other's
    // definition first, then Other's exposing list, as Other comes first,
    // then Main's import and use.
    let found = references(
        &snapshot,
        &index,
        &path,
        offset(NAVIGATED, "helper", 1),
        true,
    );
    let found: Vec<(bool, &str)> = (found.iter())
        .map(|l| {
            let text = if l.path == other { OTHER } else { NAVIGATED };
            (l.path == other, at(text, l.range))
        })
        .collect();
    assert_eq!(
        found,
        [
            (true, "helper"),
            (true, "helper"),
            (false, "helper"),
            (false, "helper")
        ]
    );
    // `shape` in `area`, where it binds and where it's used.
    let found = highlights(&snapshot, &index, &path, offset(NAVIGATED, "shape", 0));
    let found: Vec<&str> = found.iter().map(|&r| at(NAVIGATED, r)).collect();
    assert_eq!(found, ["shape", "shape"]);
}

#[test]
fn hover_shows_definitions_as_elm_language_server_does() {
    let (_dir, vfs, path, _other) = navigated();
    let (_store_dir, store) = store();
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, &path, &mut Default::default()).unwrap();
    // `area`, which has no annotation: its inferred type, then its name
    // and parameters.
    let found = hover(&snapshot, &index, &path, offset(NAVIGATED, "area", 1)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\narea : Shape -> Float\n```\n\n\n---\n\n\n```elm\narea shape\n```\n"
    );
    // `size` in the body, a variable of `let`.
    let found = hover(&snapshot, &index, &path, offset(NAVIGATED, "size", 1)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nsize : number\n```\n\n\n---\n\nDefined in local let scope"
    );
    // `r`, a case branch's variable: a parameter, linked to where its
    // type is declared.
    let found = hover(&snapshot, &index, &path, offset(NAVIGATED, "r * r", 0)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nr : Float\n```\n\n\n---\n\nLocal parameter"
    );
    let (name, file, _) = found.link.unwrap();
    assert_eq!(name, "Float");
    assert!(file.ends_with("src/Basics.elm"), "{file:?}");
}

#[test]
fn completion_after_a_qualifier_offers_what_its_module_exposes() {
    let (_dir, vfs, path, _other) = navigated();
    let (_store_dir, store) = store();
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, &path, &mut Default::default()).unwrap();
    // After `Other.`: what Other exposes, with its type.
    let at_twice = offset(NAVIGATED, "twice", 0) - TextSize::new(1);
    let Some(crate::Completions::List(found)) = completions(&snapshot, &index, &path, at_twice)
    else {
        panic!("a list of its own");
    };
    let labels: Vec<&str> = found.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["helper", "twice"]);
    assert_eq!(
        found[1].label_detail.as_deref(),
        Some(" : number -> number")
    );
    assert_eq!(found[1].sort_text.as_deref(), Some("a_twice"));
}
