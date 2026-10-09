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
    fs::write(dir.path().join("elm.json"), APP).unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/Main.elm");
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
