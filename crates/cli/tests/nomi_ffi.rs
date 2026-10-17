//! nomi/core's `Nomi.Ffi`, bundled with nomi: a Node entry built with
//! `--ffi` imports the module from where its output is, and each call
//! ends as the function does, its interruption aborting the function's
//! signal; without an FFI module, or for output that can't import one,
//! the build fails. A literal name the module doesn't export fails the
//! build, and one the build can't confirm fails the program when it
//! starts; a name made at run time fails its call.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use website::offline;

/// A program that calls each of [`FFI`]'s functions, printing how each
/// call ends; the one that never ends is timed out, and the last call
/// reports whether its signal was aborted.
const PROGRAM: &str = r#"module Main exposing (main)

import Json.Decode as Decode
import Json.Encode as Encode
import Nomi
import Nomi.Ffi as Ffi
import Nomi.Task
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \_ ->
            Task.sequence
                [ show "success" (Ffi.task "add" pair Decode.int)
                , show "decoder failure" (Ffi.task "add" pair Decode.string)
                , show "throw" (Ffi.task "throws" Encode.null Decode.int)
                , show "rejection" (Ffi.task "rejects" Encode.null Decode.int)
                , show "missing" (Ffi.task ("miss" ++ "ing") Encode.null Decode.int)
                , show "interrupted" (Nomi.Task.timeout 10 (Ffi.task "hangs" Encode.null Decode.int))
                , show "aborted" (Ffi.task "aborted" Encode.null Decode.string)
                ]
                |> Task.map (\_ -> ())


pair : Encode.Value
pair =
    Encode.list Encode.int [ 1, 2 ]


show : String -> Task x a -> Task y ()
show label task =
    task
        |> Task.map Ok
        |> Task.onError (Task.succeed << Err)
        |> Task.map (\result -> Debug.log label result |> always ())
"#;

/// The FFI module: a sum, a throw, a rejection, and a call that never
/// ends but keeps its signal for `aborted` to read.
const FFI: &str = r#"let kept = null;

export async function add([a, b]) {
  return a + b;
}

export function throws() {
  throw new Error("thrown");
}

export async function rejects() {
  throw new TypeError("rejected");
}

export function hangs(_, signal) {
  kept = signal;
  return new Promise(() => {});
}

export async function aborted() {
  return `${kept.aborted}, ${kept.reason.name}`;
}
"#;

/// An application of [`PROGRAM`] on nomi/core, with [`FFI`] at
/// `src/ffi.mjs`, in a new directory.
fn project() -> (tempfile::TempDir, PathBuf) {
    project_of(PROGRAM, FFI)
}

/// An application of `program` on nomi/core, with `ffi` at `src/ffi.mjs`,
/// in a new directory.
fn project_of(program: &str, ffi: &str) -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), program).unwrap();
    fs::write(root.join("src/ffi.mjs"), ffi).unwrap();
    (work, root)
}

/// `nomi build src/Main.elm` with `args` in `root`, offline from an empty
/// store, so from the bundle.
fn build(root: &Path, args: &[&str]) -> Output {
    let cache = website::fixture_cache();
    let program = Path::new(env!("CARGO_BIN_EXE_nomi"));
    let mut build = offline(program, cache.path());
    build
        .current_dir(root)
        .args(["build", "src/Main.elm"])
        .args(args);
    build.output().unwrap()
}

/// What the build said, on either stream.
fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn each_ffi_call_ends_as_its_function_does_and_interruption_aborts_its_signal() {
    nomi_testkit::require_node();
    let (_work, root) = project();
    let built = build(&root, &["--output=dist/main.mjs", "--ffi=src/ffi.mjs"]);
    assert!(built.status.success(), "{built:?}");
    let module = fs::read_to_string(root.join("dist/main.mjs")).unwrap();
    assert!(
        module.starts_with("import * as __nomi_ffi from '../src/ffi.mjs';\n"),
        "{}",
        &module[..200]
    );
    // From another directory, as the import is the output's own.
    let ran = Command::new(nomi_testkit::node())
        .arg(root.join("dist/main.mjs"))
        .current_dir(root.parent().unwrap())
        .output()
        .unwrap();
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "success: Ok 3\n\
         decoder failure: Err (BadResult \"Problem with the given value:\\n\\n3\\n\\nExpecting a STRING\")\n\
         throw: Err (Threw \"thrown\")\n\
         rejection: Err (Threw \"rejected\")\n\
         missing: Err (Threw \"the FFI module exports no function `missing`\")\n\
         interrupted: Ok Nothing\n\
         aborted: Ok \"true, Interrupted\"\n"
    );
}

#[test]
fn a_build_without_an_ffi_module_names_the_path_to_the_ffi() {
    let (_work, root) = project();
    let built = build(&root, &["--output=dist/main.mjs"]);
    assert!(!built.status.success(), "{built:?}");
    let said = said(&built);
    assert!(said.contains("NO FFI MODULE"), "{said}");
    assert!(
        said.contains(
            "The `main` in Main calls JavaScript through Nomi.Ffi, but this build has no FFI\n\
             module to call:\n\n    main → Nomi.Ffi.task → Elm.Kernel.NomiFfi\n"
        ),
        "{said}"
    );
    assert!(!root.join("dist").exists());
}

#[test]
fn an_ffi_module_needs_node_and_an_es_module() {
    let (_work, root) = project();
    let iife = build(&root, &["--output=dist/main.js", "--ffi=src/ffi.mjs"]);
    assert!(!iife.status.success(), "{iife:?}");
    assert!(said(&iife).contains("FFI NEEDS AN ES MODULE"), "{iife:?}");
    let browser = build(
        &root,
        &[
            "--platform=browser",
            "--output=dist/main.mjs",
            "--ffi=src/ffi.mjs",
        ],
    );
    assert!(!browser.status.success(), "{browser:?}");
    assert!(said(&browser).contains("FFI NEEDS NODE"), "{browser:?}");
    assert!(!root.join("dist").exists());
}

/// A program whose `main` prints how `Page.load` ends.
const MAIN: &str = r#"module Main exposing (main)

import Nomi
import Page
import Task


main : Nomi.Program
main =
    Nomi.program <|
        \_ ->
            Page.load
                |> Task.map Ok
                |> Task.onError (Task.succeed << Err)
                |> Task.map (\result -> Debug.log "result" result |> always ())
"#;

/// An application of [`MAIN`], whose `Page.load` is `call` given a pair
/// and decoding an `Int`, with `ffi` at `src/ffi.mjs` and `files` beside
/// it.
fn paged(call: &str, ffi: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let (work, root) = project_of(MAIN, ffi);
    let page = format!(
        "module Page exposing (load)\n\n\
         import Json.Decode as Decode\n\
         import Json.Encode as Encode\n\
         import Nomi.Ffi as Ffi\n\
         import Task exposing (Task)\n\n\n\
         load : Task Ffi.Error Int\n\
         load =\n    {call} (Encode.list Encode.int [ 1, 2 ]) Decode.int\n"
    );
    fs::write(root.join("src/Page.elm"), page).unwrap();
    for (path, text) in files {
        fs::write(root.join(path), text).unwrap();
    }
    (work, root)
}

/// The built `dist/main.mjs` in `root`, run by `node`.
fn run(root: &Path) -> Output {
    Command::new(nomi_testkit::node())
        .arg(root.join("dist/main.mjs"))
        .output()
        .unwrap()
}

/// The build of an application of [`paged`] with its FFI module, which
/// has to succeed, and how it runs.
fn built_and_run(call: &str, ffi: &str, files: &[(&str, &str)]) -> Output {
    let (_work, root) = paged(call, ffi, files);
    let built = build(&root, &["--output=dist/main.mjs", "--ffi=src/ffi.mjs"]);
    assert!(built.status.success(), "{built:?}");
    run(&root)
}

#[test]
fn a_misspelt_literal_name_fails_the_build_naming_its_caller_and_the_exports() {
    let (_work, root) = paged(r#"Ffi.task "ad""#, FFI, &[]);
    let built = build(&root, &["--output=dist/main.mjs", "--ffi=src/ffi.mjs"]);
    assert!(!built.status.success(), "{built:?}");
    let said = said(&built);
    assert!(said.contains("UNKNOWN FFI FUNCTION"), "{said}");
    assert!(
        said.contains(
            "This call names a function the FFI module does not export:\n\n    \
             Page.load calls Nomi.Ffi.task \"ad\"\n\n\
             It exports:\n\n    aborted, add, hangs, rejects, throws\n"
        ),
        "{said}"
    );
    assert!(!root.join("dist").exists());
}

#[test]
fn a_missing_ffi_module_fails_the_build() {
    let (_work, root) = project();
    let built = build(&root, &["--output=dist/main.mjs", "--ffi=src/missing.mjs"]);
    assert!(!built.status.success(), "{built:?}");
    let said = said(&built);
    assert!(said.contains("FFI MODULE NOT FOUND"), "{said}");
    assert!(
        said.contains("The FFI module given with --ffi, src/missing.mjs, does not exist."),
        "{said}"
    );
    assert!(!root.join("dist").exists());
}

#[test]
fn a_name_made_at_run_time_is_checked_when_called() {
    nomi_testkit::require_node();
    let ffi = format!("{FFI}export const version = 1;\n");
    let ran = built_and_run(r#"Ffi.task ("a" ++ "dd")"#, &ffi, &[]);
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "result: Ok 3\n");
    let ran = built_and_run(r#"Ffi.task ("ver" ++ "sion")"#, &ffi, &[]);
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "result: Err (Threw \"the FFI module exports no function `version`\")\n"
    );
}

#[test]
fn a_module_exporting_all_of_another_builds_and_its_names_are_checked_when_it_starts() {
    nomi_testkit::require_node();
    let ffi = "export * from './impl.mjs';\n";
    let files = [("src/impl.mjs", FFI)];
    let ran = built_and_run(r#"Ffi.task "add""#, ffi, &files);
    assert_eq!(ran.status.code(), Some(0), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "result: Ok 3\n");
    let ran = built_and_run(r#"Ffi.task "ad""#, ffi, &files);
    assert_eq!(ran.status.code(), Some(1), "{ran:?}");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "");
    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert!(
        stderr.contains("Error: the FFI module exports no function `ad`, which Page.load calls"),
        "{stderr}"
    );
}
