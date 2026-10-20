//! nomi/sql on SQLite files, bundled with nomi, on Bun: a database opened
//! on the disk under its capability's root keeps what was written for
//! the next open; opened under `Fs.readOnly` it reads and refuses
//! writes; a path out of the root, or into a directory that isn't there,
//! fails as `Fs` does; and a file that isn't a database is refused when
//! it is opened. The rest of `Nomi.Sql` is the `sql` suite's, in memory.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use website::offline;

/// A program that opens `data/todos.db` under the working directory, or
/// under it read-only with `read-only`, or the path `open` names, and
/// runs each of its other arguments as a statement, printing the rows a
/// query gives, as `[(name, value)]`, or the error, a line for each.
const PROGRAM: &str = r#"module Main exposing (main)

import Nomi exposing (Env)
import Nomi.Fs as Fs
import Nomi.Sql as Sql
import Nomi.Stdio as Stdio
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                "read-only" :: statements ->
                    run env (Fs.readOnly env.fs) "data/todos.db" statements

                "open" :: path :: statements ->
                    run env env.fs path statements

                statements ->
                    run env env.fs "data/todos.db" statements


run : Env -> Fs.Fs -> String -> List String -> Task String ()
run env fs path statements =
    Sql.open path fs
        |> Task.andThen (\db -> Task.sequence (List.map (\statement -> Sql.query statement [] Sql.columns db) statements))
        |> Task.map (List.map Debug.toString >> String.join "\n")
        |> Task.onError (\error -> Task.succeed (Debug.toString error))
        |> Task.andThen (\out -> Stdio.printLine out env.stdout |> Task.mapError Debug.toString)
"#;

/// The program built from an empty store, offline, so from the bundle,
/// with `data/` beside it to run in.
fn built() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/sql": "1.0.0"}, "indirect": {"elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), PROGRAM).unwrap();
    let cache = website::fixture_cache();
    let built = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args(["build", "src/Main.elm", "--output=dist/main.mjs"])
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    fs::create_dir(root.join("data")).unwrap();
    (work, root)
}

/// What the program printed, run in `root` with `args`.
fn ran(root: &Path, args: &[&str]) -> String {
    let output: Output = Command::new(nomi_testkit::bun())
        .arg(root.join("dist/main.mjs"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn a_database_on_disk_keeps_what_was_written_for_the_next_open() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    assert_eq!(
        ran(
            &root,
            &[
                "create table todos (title text)",
                "insert into todos values ('milk')",
                "select title from todos"
            ]
        ),
        "[]\n[]\n[[(\"title\",Text \"milk\")]]\n"
    );
    assert!(root.join("data/todos.db").is_file());
    assert_eq!(
        ran(&root, &["select count(*) as n from todos"]),
        "[[(\"n\",Int 1)]]\n"
    );
}

#[test]
fn read_only_a_database_reads_and_refuses_writes() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    ran(
        &root,
        &[
            "create table todos (title text)",
            "insert into todos values ('milk')",
        ],
    );
    assert_eq!(
        ran(&root, &["read-only", "select title from todos"]),
        "[[(\"title\",Text \"milk\")]]\n"
    );
    assert_eq!(
        ran(&root, &["read-only", "insert into todos values ('eggs')"]),
        "Failed { code = \"SQLITE_READONLY\", message = \"attempt to write a readonly database\" }\n"
    );
    fs::remove_file(root.join("data/todos.db")).unwrap();
    assert_eq!(
        ran(&root, &["read-only"]),
        "FileError (NotFound \"data/todos.db\")\n"
    );
    assert!(!root.join("data/todos.db").exists());
}

#[test]
fn a_path_out_of_the_root_or_into_no_directory_fails_as_fs_does() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    assert_eq!(
        ran(&root, &["open", "../todos.db"]),
        "FileError (OutsideRoot \"../todos.db\")\n"
    );
    assert_eq!(
        ran(&root, &["open", "nowhere/todos.db"]),
        "FileError (NotFound \"nowhere/todos.db\")\n"
    );
    assert_eq!(
        ran(&root, &["open", "data"]),
        "FileError (IsADirectory \"data\")\n"
    );
    std::os::unix::fs::symlink("/tmp", root.join("data/out")).unwrap();
    assert_eq!(
        ran(&root, &["open", "data/out/todos.db"]),
        "FileError (OutsideRoot \"data/out/todos.db\")\n"
    );
}

#[test]
fn a_file_that_isnt_a_database_is_refused_as_it_opens() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    fs::write(
        root.join("data/notes.txt"),
        "these are notes, not a database, and SQLite reads their first hundred bytes as a header",
    )
    .unwrap();
    assert_eq!(
        ran(&root, &["open", "data/notes.txt"]),
        "Failed { code = \"SQLITE_NOTADB\", message = \"file is not a database\" }\n"
    );
}
