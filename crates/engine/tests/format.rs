//! `nomi format` against a real directory.

use std::fs;
use std::path::Path;

use nomi_base::ElmPosition;
use nomi_engine::Vfs;
use nomi_engine::format::{Mode, Outcome, RootError, format_paths, format_text};

const FORMATTED: &str = "module A exposing (x)\n\n\nx =\n    1\n";
const UNFORMATTED: &str = "module A exposing (x)\nx  =  1\n";
const INVALID: &str = "module A exposing (x)\nx = (\n";

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap()
}

/// Each file's name and outcome, as a short word.
fn outcomes(root: &Path, files: &[(std::path::PathBuf, Outcome)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(path, outcome)| {
            // By `/`-separated components, as on every platform.
            let name = (path.strip_prefix(root).unwrap().components())
                .map(|c| c.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            let word = match outcome {
                Outcome::Unchanged => "unchanged".to_owned(),
                Outcome::Formatted => "formatted".to_owned(),
                Outcome::Invalid(at) => format!("invalid {}:{}", at.row, at.column),
                Outcome::Failed(e) => format!("failed {:?}", e.kind()),
            };
            (name, word)
        })
        .collect()
}

#[test]
fn text_is_formatted_or_placed() {
    assert_eq!(format_text(UNFORMATTED).as_deref(), Ok(FORMATTED));
    assert_eq!(format_text(INVALID), Err(ElmPosition { row: 3, column: 1 }));
}

#[test]
fn write_mode_rewrites_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "A.elm", FORMATTED);
    write(root, "B.elm", UNFORMATTED);
    write(root, "C.elm", INVALID);
    let report = format_paths(&Vfs::new(), &[root.to_owned()], Mode::Write);
    assert!(report.roots.is_empty());
    assert_eq!(
        outcomes(root, &report.files),
        [
            ("A.elm".to_owned(), "unchanged".to_owned()),
            ("B.elm".to_owned(), "formatted".to_owned()),
            ("C.elm".to_owned(), "invalid 3:1".to_owned()),
        ]
    );
    assert_eq!(read(root, "B.elm"), FORMATTED);
    assert_eq!(read(root, "C.elm"), INVALID);
    assert!(!report.succeeded(Mode::Write));
}

#[test]
fn check_mode_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "A.elm", UNFORMATTED);
    let report = format_paths(&Vfs::new(), &[root.to_owned()], Mode::Check);
    assert_eq!(
        outcomes(root, &report.files),
        [("A.elm".to_owned(), "formatted".to_owned())]
    );
    assert_eq!(read(root, "A.elm"), UNFORMATTED);
    assert!(!report.succeeded(Mode::Check));
    assert!(report.succeeded(Mode::Write));
}

#[test]
fn a_module_named_twice_is_formatted_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "src/A.elm", UNFORMATTED);
    let roots = [root.to_owned(), root.join("src"), root.join("src/./A.elm")];
    let report = format_paths(&Vfs::new(), &roots, Mode::Write);
    assert_eq!(
        outcomes(root, &report.files),
        [("src/A.elm".to_owned(), "formatted".to_owned())]
    );
}

#[test]
fn roots_without_modules_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "empty/notes.txt", "");
    write(root, "A.elm", FORMATTED);
    let roots = [root.join("empty"), root.join("missing"), root.join("A.elm")];
    let report = format_paths(&Vfs::new(), &roots, Mode::Check);
    let errors: Vec<_> = report
        .roots
        .iter()
        .map(|(path, error)| (path.strip_prefix(root).unwrap().to_owned(), error))
        .collect();
    assert!(matches!(errors[..], [
        (ref empty, RootError::Empty),
        (ref missing, RootError::Io(_)),
    ] if empty == Path::new("empty") && missing == Path::new("missing")));
    assert_eq!(report.roots[0].1.to_string(), "no .elm files");
    assert_eq!(report.files.len(), 1);
    assert!(!report.succeeded(Mode::Check));
}

#[test]
fn many_modules_keep_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let names: Vec<_> = (0..200).map(|i| format!("M{i:03}.elm")).collect();
    for name in &names {
        write(root, name, UNFORMATTED);
    }
    let report = format_paths(&Vfs::new(), &[root.to_owned()], Mode::Write);
    let found: Vec<_> = outcomes(root, &report.files)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(found, names);
    assert!(names.iter().all(|name| read(root, name) == FORMATTED));
}
