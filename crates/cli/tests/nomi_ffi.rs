//! nomi/core's `Nomi.Ffi`, bundled with nomi: a Node entry built with
//! `--ffi` imports the module from where its output is, and each call
//! ends as the function does, its interruption aborting the function's
//! signal; without an FFI module, or for output that can't import one,
//! the build fails.
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
                , show "missing" (Ffi.task "missing" Encode.null Decode.int)
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
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0"}, "indirect": {"elm/bytes": "1.0.8"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), PROGRAM).unwrap();
    fs::write(root.join("src/ffi.mjs"), FFI).unwrap();
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
    let ran = Command::new("node")
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
