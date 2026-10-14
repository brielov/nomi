use std::fs;
use std::path::{Path, PathBuf};

use nomi_engine::index::{Index, PackageModules};
use nomi_engine::{Snapshot, Vfs};
use nomi_testkit::packages::CORE_AND_JSON_APP;

use super::*;

/// A project with `module` at `src/Main.elm`, open in an editor.
fn open(module: &str) -> (tempfile::TempDir, Vfs, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    // Found files have their canonical names, which differ where the
    // temporary directory is behind a link, as on macOS.
    let root = nomi_engine::canonical(dir.path()).unwrap();
    fs::write(root.join("elm.json"), CORE_AND_JSON_APP).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let path = root.join("src/Main.elm");
    let vfs = Vfs::new();
    vfs.set_overlay(&path, module).unwrap();
    (dir, vfs, path)
}

fn at(text: &str, range: TextRange) -> &str {
    &text[range]
}

/// `text` with `edits` made.
fn apply(text: &str, edits: &[Edit]) -> String {
    let mut after = text.to_owned();
    for edit in edits.iter().rev() {
        after.replace_range(std::ops::Range::<usize>::from(edit.range), &edit.text);
    }
    after
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
    assert_eq!(context.source, nomi_syntax::Source::Application);
    let outside = Context::of(&vfs.snapshot(), Path::new("/nowhere/Main.elm"));
    assert_eq!(outside.compat, Compat::LATEST);
    drop(dir);
}

#[test]
fn positions_count_as_the_release_does() {
    let text = "ab\r\ncd😀e\n";
    let offset = |row, col, compat| {
        let at = Position { row, col };
        let region = Region { start: at, end: at };
        elm_range(text, &LineIndex::new(text), region, compat).start()
    };
    // 0.19.1 counts from 1, and a carriage return is no column.
    assert_eq!(offset(1, 3, Compat::V0_19_1), TextSize::new(2));
    assert_eq!(offset(2, 4, Compat::V0_19_1), TextSize::new(10));
    assert_eq!(offset(1, 3, Compat::V0_19_3), TextSize::new(10));
    assert_eq!(offset(9, 0, Compat::V0_19_3), TextSize::of(text));
    assert_eq!(offset(0, 99, Compat::V0_19_3), TextSize::new(3));
}

#[test]
fn formatting_edits_only_what_changes() {
    let before = "module Main exposing (x, y)\n\n\nx = 1\n\n\n\n\ny =\n  2\n";
    let (_dir, vfs, path) = open(before);
    let edits = format(&vfs.snapshot(), &path).unwrap().unwrap();
    assert_eq!(apply(before, &edits), nomi_format::format(before).unwrap());
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

/// The project of the file at `path`, as `vfs` has it, indexed with a
/// package store holding the fixture copy of elm/core 1.0.5.
struct Project {
    snapshot: Snapshot,
    index: Index,
    store: nomi_engine::Store,
    _store_dir: tempfile::TempDir,
}

fn indexed(vfs: &Vfs, path: &Path) -> Project {
    let store_dir = tempfile::tempdir().unwrap();
    let hash = "0".repeat(40);
    let core = nomi_testkit::packages::dir().join("elm/core/1.0.5");
    let sources = store_dir.path().join("sources").join(&hash);
    nomi_testkit::copy_tree(&core, &sources, |_| true).unwrap();
    let release = store_dir.path().join("releases/elm/core");
    fs::create_dir_all(&release).unwrap();
    fs::write(release.join("1.0.5"), format!("{hash}\n")).unwrap();
    let store = nomi_engine::Store::new(store_dir.path().to_owned());
    let snapshot = vfs.snapshot();
    let index =
        nomi_engine::index::index(&snapshot, &store, path, &mut Default::default()).unwrap();
    Project {
        snapshot,
        index,
        store,
        _store_dir: store_dir,
    }
}

/// One of elm's reports, at `range`.
fn elm_report(code: Code, message: &str, range: TextRange) -> Diagnosed {
    Diagnosed {
        code: DiagnosticCode::Known(code),
        message: message.to_owned(),
        range,
        severity: Severity::Error,
        source: Some(Source::Elm),
        unnecessary: false,
        fixes: Vec::new(),
    }
}

/// A code action request about `given` at its range.
fn request<'a>(given: &'a [Diagnosed], packages: &'a PackageModules) -> Request<'a> {
    Request {
        range: given[0].range,
        given,
        problems: &[],
        packages,
        resolve: Resolve::Now,
        moving: Moving::Unsupported,
    }
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
fn navigated() -> (tempfile::TempDir, Vfs, PathBuf, PathBuf) {
    let (dir, vfs, path) = open(NAVIGATED);
    let other = nomi_engine::canonical(dir.path())
        .unwrap()
        .join("src/Other.elm");
    fs::write(&other, OTHER).unwrap();
    (dir, vfs, path, other)
}

/// The offset of the `n`th `needle` in `text`, plus one.
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
    std::os::unix::fs::symlink(nomi_engine::canonical(dir.path()).unwrap(), &link).unwrap();
    let path = link.join("src/Main.elm");
    let vfs = Vfs::new();
    vfs.set_overlay(&path, NAVIGATED).unwrap();
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let found = definition(snapshot, index, &path, offset(NAVIGATED, "area", 0)).unwrap();
    assert_eq!(
        found.path,
        nomi_engine::canonical(dir.path())
            .unwrap()
            .join("src/Main.elm")
    );
}

#[test]
fn definition_finds_locals_top_level_names_and_other_modules() {
    let (_dir, vfs, path, other) = navigated();
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let go = |needle, n| definition(snapshot, index, &path, offset(NAVIGATED, needle, n));
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
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    // `helper`, from its use in Main: with the declaration, Other's
    // definition first, then Other's exposing list, as Other comes first,
    // then Main's import and use.
    let found = references(
        snapshot,
        index,
        &path,
        offset(NAVIGATED, "helper", 1),
        Declaration::Include,
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
    let found = highlights(snapshot, index, &path, offset(NAVIGATED, "shape", 0));
    let found: Vec<&str> = found.iter().map(|&r| at(NAVIGATED, r)).collect();
    assert_eq!(found, ["shape", "shape"]);
}

#[test]
fn hover_shows_definitions_as_elm_language_server_does() {
    let (_dir, vfs, path, _other) = navigated();
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    // `area`, which has no annotation: its inferred type, then its name
    // and parameters.
    let found = hover(snapshot, index, &path, offset(NAVIGATED, "area", 1)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\narea : Shape -> Float\n```\n\n\n---\n\n\n```elm\narea shape\n```\n"
    );
    // `size` in the body, a variable of `let`.
    let found = hover(snapshot, index, &path, offset(NAVIGATED, "size", 1)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nsize : number\n```\n\n\n---\n\nDefined in local let scope"
    );
    // `r`, a case branch's variable: a parameter, linked to where its
    // type is declared.
    let found = hover(snapshot, index, &path, offset(NAVIGATED, "r * r", 0)).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nr : Float\n```\n\n\n---\n\nLocal parameter"
    );
    let link = found.link.unwrap();
    assert_eq!(link.name, "Float");
    assert!(link.path.ends_with("src/Basics.elm"), "{:?}", link.path);
}

#[test]
fn completion_after_a_qualifier_offers_what_its_module_exposes() {
    let (_dir, vfs, path, _other) = navigated();
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    // After `Other.`: what Other exposes, with its type.
    let at_twice = offset(NAVIGATED, "twice", 0) - TextSize::new(1);
    let Some(Completions::Items(found)) = completions(snapshot, index, &path, at_twice) else {
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

#[test]
fn a_missing_module_offers_the_packages_that_expose_it() {
    let text = "module Main exposing (main)\n\nimport Dict\n\n\nmain =\n    Dict.empty\n";
    let (_dir, vfs, path) = open(text);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot,
        index,
        store,
        ..
    } = &project;
    let at = text.find("Dict").unwrap() as u32;
    let message = "MODULE NOT FOUND - You are trying to import a `Dict` module:";
    let range = TextRange::new(TextSize::new(at), TextSize::new(at + 4));
    let given = [elm_report(Code::ImportResolve, message, range)];
    let packages = PackageModules::read(store);
    let found = code_actions(snapshot, index, &path, &request(&given, &packages));
    let offered: Vec<_> = (found.iter())
        .map(|a| (a.title.as_str(), a.command.clone()))
        .collect();
    assert_eq!(
        offered,
        [(
            "Install package \"elm/core\"",
            Some(ActionCommand::InstallPackage {
                package: "elm/core".to_owned()
            })
        )]
    );
    // A module no package downloaded has: nothing to install.
    let missing = TextRange::new(TextSize::new(at), TextSize::new(at + 2));
    let given = [elm_report(Code::ImportResolve, message, missing)];
    let found = code_actions(snapshot, index, &path, &request(&given, &packages));
    assert_eq!(found, []);
}

#[test]
fn exposing_edits_keep_the_list_well_formed() {
    let text = "module Main exposing (a, B(..), c)\n\n\na =\n    1\n";
    let (_dir, vfs, path) = open(text);
    let snapshot = vfs.snapshot();
    let edit = |name, exposure| {
        let edit = exposing_edit(&snapshot, &path, name, exposure)?;
        Some(apply(text, &[edit]).lines().next().unwrap().to_owned())
    };
    let (expose, unexpose) = (Exposure::Expose, Exposure::Unexpose);
    // The first and middle ones take the comma after them, the last the
    // one before it.
    assert_eq!(
        edit("a", unexpose).as_deref(),
        Some("module Main exposing (B(..), c)")
    );
    assert_eq!(
        edit("B", unexpose).as_deref(),
        Some("module Main exposing (a, c)")
    );
    assert_eq!(
        edit("c", unexpose).as_deref(),
        Some("module Main exposing (a, B(..))")
    );
    assert_eq!(
        edit("d", expose).as_deref(),
        Some("module Main exposing (a, B(..), c, d)")
    );
    // Already exposed, or not there to unexpose.
    assert_eq!(edit("a", expose), None);
    assert_eq!(edit("d", unexpose), None);
    // The only one exposed stays, as elm-format would add it back.
    let (_dir, vfs, path) = open("module Main exposing (a)\n\n\na =\n    1\n");
    assert_eq!(exposing_edit(&vfs.snapshot(), &path, "a", unexpose), None);
}

#[test]
fn a_resolved_annotation_is_inferred_at_the_definitions_column() {
    let text = "module Main exposing (main)\n\n\nmain =\n    let\n        twice n =\n            n * 2\n    in\n    twice 2\n";
    let (_dir, vfs, path) = open(text);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let at = TextSize::new(text.find("twice n").unwrap() as u32 + 1);
    let action = RefactorAction::AddTypeAnnotation;
    let (edits, rename) =
        refactor_edits(index, snapshot, &path, action, TextRange::empty(at)).unwrap();
    assert_eq!(rename, None);
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].text, "twice : number -> number\n        ");
    assert_eq!(edits[0].range, TextRange::empty(at - TextSize::new(1)));
}

/// The actions for one of elm's reports, at `region` in `text`.
fn actions_for(text: &str, region: &str, message: &str) -> (String, Vec<Action>) {
    let (_dir, vfs, path) = open(text);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let at = text.find(region).unwrap() as u32;
    let range = TextRange::new(TextSize::new(at), TextSize::new(at + region.len() as u32));
    let given = [elm_report(Code::ElmMake, message, range)];
    let packages = PackageModules::default();
    let found = code_actions(snapshot, index, &path, &request(&given, &packages));
    (text.to_owned(), found)
}

/// `text` with an action's edits of its first file made.
fn applied(text: &str, action: &Action) -> String {
    let (_, edits) = action.edits.iter().flatten().next().unwrap();
    apply(text, edits)
}

#[test]
fn elms_missing_patterns_get_their_branches() {
    let text = "module Main exposing (main)\n\n\ntype Foo\n    = Bar\n    | Biz\n    | Baz\n\n\nmain : Foo -> ()\nmain foo =\n    case foo of\n        Bar ->\n            ()\n";
    let message = "MISSING PATTERNS - This `case` does not have branches for all possibilities:\n\n12|#>#    case foo of\n\nMissing possibilities include:\n\n    #Biz#\n    #Baz#\n\nI would have to crash if I saw one of those. Add branches for them!";
    let region = "case foo of\n        Bar ->\n            ()";
    let (text, found) = actions_for(text, region, message);
    let titles: Vec<_> = found.iter().map(|a| a.title.as_str()).collect();
    // The range is the whole `case`, so it can be extracted too.
    assert_eq!(
        titles,
        ["Add missing case branches", "Extract function to top level"]
    );
    assert_eq!(
        applied(&text, &found[0]),
        "module Main exposing (main)\n\n\ntype Foo\n    = Bar\n    | Biz\n    | Baz\n\n\nmain : Foo -> ()\nmain foo =\n    case foo of\n        Bar ->\n            ()\n\n        Biz ->\n            Debug.todo \"branch 'Biz' not implemented\"\n\n        Baz ->\n            Debug.todo \"branch 'Baz' not implemented\"\n"
    );
}

#[test]
fn a_missing_field_goes_into_its_alias_typed_by_its_use() {
    let text = "module Main exposing (main)\n\n\ntype alias Model =\n    { count : Int\n    }\n\n\nmain : Model -> Float\nmain model =\n    model.ratio\n";
    let message = "TYPE MISMATCH - This `model` record does not have a `ratio` field:";
    let (text, found) = actions_for(text, "model.ratio", message);
    let titles: Vec<_> = found.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Create missing record field",
            "Extract function to top level"
        ]
    );
    assert_eq!(
        applied(&text, &found[0]),
        "module Main exposing (main)\n\n\ntype alias Model =\n    { count : Int\n    , ratio : Float\n    }\n\n\nmain : Model -> Float\nmain model =\n    model.ratio\n"
    );
    // On one line, after the last field.
    let text = "module Main exposing (main)\n\n\ntype alias Model = { count : Int }\n\n\nmain : Model -> Float\nmain model =\n    model.ratio\n";
    let (text, found) = actions_for(text, "model.ratio", message);
    assert_eq!(
        applied(&text, &found[0]),
        "module Main exposing (main)\n\n\ntype alias Model = { count : Int, ratio : Float }\n\n\nmain : Model -> Float\nmain model =\n    model.ratio\n"
    );
}

#[test]
fn a_qualified_type_is_one_name() {
    // As tree-sitter-elm reads it: from its qualifier on, `Dict.Dict` is
    // the type, where a value's qualifier would be its module.
    let main =
        "module Main exposing (x)\n\nimport Dict\n\n\nx : Dict.Dict Int Int -> Int\nx v =\n    1\n";
    let (_dir, vfs, path) = open(main);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let qualifier = TextSize::new(main.find("Dict.Dict").unwrap() as u32);
    let name = qualifier + TextSize::new(5);
    let found = definition(snapshot, index, &path, qualifier).unwrap();
    assert_eq!(Some(found), definition(snapshot, index, &path, name));
    let hovered = hover(snapshot, index, &path, qualifier).unwrap();
    assert!(
        hovered.markdown.contains("type Dict k v"),
        "{}",
        hovered.markdown
    );
}

#[test]
fn an_alias_pattern_is_typed_as_its_pattern() {
    let main = "module Main exposing (f)\n\n\nf : { a : Int, b : String } -> Int\nf ({ a } as whole) =\n    a\n";
    let (_dir, vfs, path) = open(main);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let at = TextSize::new(main.find("whole").unwrap() as u32 + 1);
    let found = hover(snapshot, index, &path, at).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nwhole : { a : Int, b : String }\n```\n\n\n---\n\nLocal parameter"
    );
}

#[test]
fn a_call_is_typed_through_its_last_arguments_parentheses() {
    // elm's region for `matrix (size - 1)` ends before the `)`.
    let main = "module Main exposing (matrix)\n\n\nmatrix : Int -> List Int\nmatrix size =\n    if size <= 0 then\n        []\n\n    else\n        let\n            subMatrix =\n                matrix (size - 1)\n        in\n        subMatrix\n";
    let (_dir, vfs, path) = open(main);
    let project = indexed(&vfs, &path);
    let Project {
        snapshot, index, ..
    } = &project;
    let at = TextSize::new(main.find("subMatrix").unwrap() as u32 + 1);
    let found = hover(snapshot, index, &path, at).unwrap();
    assert_eq!(
        found.markdown,
        "\n```elm\nsubMatrix : List Int\n```\n\n\n---\n\nDefined in local let scope"
    );
}
