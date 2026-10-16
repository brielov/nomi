//! nomi/core's capabilities, bundled with nomi: `Nomi.Fs` on disk and in
//! memory gives the same results for one table of cases, and neither `..`
//! nor a symbolic link leads out of a `subdir`; `Nomi.Stdio` reads lines
//! to the end of input; `Nomi.Process.exit` runs finalizers, then exits.
//! The fakes are reached as nomi/test reaches them, from another bundled
//! package's kernel: the fixture nomi/fake, bundled beside nomi/core.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use website::offline;

/// A program picked by its arguments: `table real` or `table memory` runs
/// the file system's table on `scratch/` or on a memory file system;
/// `links` the cases only the disk has, in `scratch/links/`; `lines`
/// reads standard input to its end; `fake-stdio` reads a scripted input
/// into a captured output; `exit` exits 3 with finalizers to run.
const PROGRAM: &str = r#"module Main exposing (main)

import Bytes
import Nomi exposing (Env)
import Nomi.Fake as Fake
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Process
import Nomi.Stdio as Stdio
import Nomi.Task
import Process
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            case env.args of
                [ "table", "real" ] ->
                    table env (Fs.subdir "scratch" env.fs)

                [ "table", "memory" ] ->
                    Fake.fs [] |> Task.andThen (table env)

                [ "links" ] ->
                    report env (links (Fs.subdir "scratch/links" env.fs))

                [ "lines" ] ->
                    lines env

                [ "fake-stdio" ] ->
                    Task.map2 Tuple.pair (Fake.reader "a\nb\r\n\nc") Fake.writer
                        |> Task.andThen
                            (\( reader, writer ) ->
                                lines { env | stdin = reader, stdout = writer }
                                    |> Task.andThen (\_ -> Fake.captured writer)
                            )
                        |> Task.andThen (\text -> Stdio.print text env.stdout)

                [ "exit" ] ->
                    Fiber.fork (Process.sleep 3600000 |> Nomi.Task.ensure (Stdio.printLine "child finalized" env.stdout))
                        |> Task.andThen (\_ -> Stdio.printLine "exiting" env.stdout)
                        |> Task.andThen (\_ -> Nomi.Process.exit 3)
                        |> Task.andThen (\_ -> Stdio.printLine "unreachable" env.stdout)
                        |> Nomi.Task.ensure (Stdio.printLine "finalized" env.stdout)

                _ ->
                    Task.fail "no such case"


table : Env -> Fs -> Task String ()
table env root =
    Fs.makeDirectory "table" root
        |> Task.mapError Fs.errorToString
        |> Task.andThen (\_ -> report env (cases (Fs.subdir "table" root)))


cases : Fs -> List (Task x String)
cases fs =
    let
        readOnly =
            Fs.readOnly fs
    in
    [ outcome "write a.txt" (Fs.write "a.txt" "hello" fs)
    , outcome "read a.txt" (Fs.read "a.txt" fs)
    , outcome "append a.txt" (Fs.append "a.txt" ", world" fs)
    , outcome "read a.txt" (Fs.read "a.txt" fs)
    , outcome "readBytes a.txt" (Fs.readBytes "a.txt" fs |> Task.map Bytes.width)
    , outcome "append new.txt" (Fs.append "new.txt" "é" fs)
    , outcome "read new.txt" (Fs.read "new.txt" fs)
    , outcome "read missing.txt" (Fs.read "missing.txt" fs)
    , outcome "makeDirectory dir/sub" (Fs.makeDirectory "dir/sub" fs)
    , outcome "makeDirectory dir" (Fs.makeDirectory "dir" fs)
    , outcome "makeDirectory dir/sub" (Fs.makeDirectory "dir/sub" fs)
    , outcome "makeDirectory ." (Fs.makeDirectory "." fs)
    , outcome "makeDirectory a.txt" (Fs.makeDirectory "a.txt" fs)
    , outcome "makeDirectory a.txt/x" (Fs.makeDirectory "a.txt/x" fs)
    , outcome "write dir/sub/b.txt" (Fs.write "dir/sub/b.txt" "b" fs)
    , outcome "list ." (Fs.list "." fs)
    , outcome "list dir/" (Fs.list "dir/" fs)
    , outcome "list a.txt" (Fs.list "a.txt" fs)
    , outcome "list missing" (Fs.list "missing" fs)
    , outcome "read dir" (Fs.read "dir" fs)
    , outcome "write dir" (Fs.write "dir" "x" fs)
    , outcome "write nope/c.txt" (Fs.write "nope/c.txt" "c" fs)
    , outcome "read a.txt/x" (Fs.read "a.txt/x" fs)
    , outcome "read dir/./../a.txt" (Fs.read "dir/./../a.txt" fs)
    , outcome "read ../table/a.txt" (Fs.read "../table/a.txt" fs)
    , outcome "write dir/../../x" (Fs.write "dir/../../x" "x" fs)
    , outcome "exists a.txt" (Fs.exists "a.txt" fs)
    , outcome "exists dir/sub" (Fs.exists "dir/sub" fs)
    , outcome "exists missing.txt" (Fs.exists "missing.txt" fs)
    , outcome "exists a.txt/x" (Fs.exists "a.txt/x" fs)
    , outcome "exists ../table" (Fs.exists "../table" fs)
    , outcome "subdir dir: read sub/b.txt" (Fs.read "sub/b.txt" (Fs.subdir "dir" fs))
    , outcome "subdir dir: read ../a.txt" (Fs.read "../a.txt" (Fs.subdir "dir" fs))
    , outcome "subdir dir/..: read a.txt" (Fs.read "a.txt" (Fs.subdir "dir/.." fs))
    , outcome "subdir ..: read a.txt" (Fs.read "a.txt" (Fs.subdir ".." fs))
    , outcome "subdir ..: exists a.txt" (Fs.exists "a.txt" (Fs.subdir ".." fs))
    , outcome "subdir missing: read x" (Fs.read "x" (Fs.subdir "missing" fs))
    , outcome "readOnly: read a.txt" (Fs.read "a.txt" readOnly)
    , outcome "readOnly: write a.txt" (Fs.write "a.txt" "x" readOnly)
    , outcome "readOnly: append a.txt" (Fs.append "a.txt" "x" readOnly)
    , outcome "readOnly: remove a.txt" (Fs.remove "a.txt" readOnly)
    , outcome "readOnly: makeDirectory d" (Fs.makeDirectory "d" readOnly)
    , outcome "readOnly, subdir dir: write b.txt" (Fs.write "b.txt" "x" (Fs.subdir "dir" readOnly))
    , outcome "read a.txt" (Fs.read "a.txt" fs)
    , outcome "remove ." (Fs.remove "." fs)
    , outcome "remove dir" (Fs.remove "dir" fs)
    , outcome "exists dir/sub/b.txt" (Fs.exists "dir/sub/b.txt" fs)
    , outcome "remove dir" (Fs.remove "dir" fs)
    , outcome "remove new.txt" (Fs.remove "new.txt" fs)
    , outcome "list ." (Fs.list "." fs)
    ]


links : Fs -> List (Task x String)
links fs =
    [ outcome "read alias.txt" (Fs.read "alias.txt" fs)
    , outcome "read secret.txt" (Fs.read "secret.txt" fs)
    , outcome "read escape/secret.txt" (Fs.read "escape/secret.txt" fs)
    , outcome "list escape" (Fs.list "escape" fs)
    , outcome "write escape/new.txt" (Fs.write "escape/new.txt" "x" fs)
    , outcome "write secret.txt" (Fs.write "secret.txt" "x" fs)
    , outcome "write dangling" (Fs.write "dangling" "x" fs)
    , outcome "append dangling" (Fs.append "dangling" "x" fs)
    , outcome "makeDirectory escape/made" (Fs.makeDirectory "escape/made" fs)
    , outcome "exists escape/secret.txt" (Fs.exists "escape/secret.txt" fs)
    , outcome "subdir escape: read secret.txt" (Fs.read "secret.txt" (Fs.subdir "escape" fs))
    , outcome "subdir up: list ." (Fs.list "." (Fs.subdir "up" fs))
    , outcome "remove secret.txt" (Fs.remove "secret.txt" fs)
    , outcome "exists secret.txt" (Fs.exists "secret.txt" fs)
    ]


outcome : String -> Task e a -> Task x String
outcome name task =
    task
        |> Task.map Ok
        |> Task.onError (Err >> Task.succeed)
        |> Task.map (\result -> name ++ ": " ++ Debug.toString result)


report : Env -> List (Task x String) -> Task x ()
report env tasks =
    Task.sequence tasks
        |> Task.andThen (\outcomes -> Stdio.print (String.concat (List.map (\line -> line ++ "\n") outcomes)) env.stdout)


lines : Env -> Task x ()
lines env =
    Stdio.line env.stdin
        |> Task.andThen
            (\line ->
                Stdio.printLine (Debug.toString line) env.stdout
                    |> Task.andThen
                        (\_ ->
                            case line of
                                Just _ ->
                                    lines env

                                Nothing ->
                                    Stdio.print "end" env.stderr
                        )
            )
"#;

/// What the table prints, on disk and in memory alike.
const TABLE: &str = r#"write a.txt: Ok ()
read a.txt: Ok "hello"
append a.txt: Ok ()
read a.txt: Ok "hello, world"
readBytes a.txt: Ok 12
append new.txt: Ok ()
read new.txt: Ok "é"
read missing.txt: Err (NotFound "missing.txt")
makeDirectory dir/sub: Ok ()
makeDirectory dir: Ok ()
makeDirectory dir/sub: Ok ()
makeDirectory .: Ok ()
makeDirectory a.txt: Err (AlreadyExists "a.txt")
makeDirectory a.txt/x: Err (Other "a.txt/x: not a directory")
write dir/sub/b.txt: Ok ()
list .: Ok ["a.txt","dir","new.txt"]
list dir/: Ok ["sub"]
list a.txt: Err (Other "a.txt: not a directory")
list missing: Err (NotFound "missing")
read dir: Err (IsADirectory "dir")
write dir: Err (IsADirectory "dir")
write nope/c.txt: Err (NotFound "nope/c.txt")
read a.txt/x: Err (Other "a.txt/x: not a directory")
read dir/./../a.txt: Ok "hello, world"
read ../table/a.txt: Err (OutsideRoot "../table/a.txt")
write dir/../../x: Err (OutsideRoot "dir/../../x")
exists a.txt: Ok True
exists dir/sub: Ok True
exists missing.txt: Ok False
exists a.txt/x: Ok False
exists ../table: Ok False
subdir dir: read sub/b.txt: Ok "b"
subdir dir: read ../a.txt: Err (OutsideRoot "../a.txt")
subdir dir/..: read a.txt: Ok "hello, world"
subdir ..: read a.txt: Err (OutsideRoot "..")
subdir ..: exists a.txt: Ok False
subdir missing: read x: Err (NotFound "x")
readOnly: read a.txt: Ok "hello, world"
readOnly: write a.txt: Err (PermissionDenied "a.txt")
readOnly: append a.txt: Err (PermissionDenied "a.txt")
readOnly: remove a.txt: Err (PermissionDenied "a.txt")
readOnly: makeDirectory d: Err (PermissionDenied "d")
readOnly, subdir dir: write b.txt: Err (PermissionDenied "b.txt")
read a.txt: Ok "hello, world"
remove .: Err (PermissionDenied ".")
remove dir: Ok ()
exists dir/sub/b.txt: Ok False
remove dir: Err (NotFound "dir")
remove new.txt: Ok ()
list .: Ok ["a.txt"]
"#;

/// A bundle of nomi/core, from `packages/`, and the fixture nomi/fake.
fn bundle(work: &Path) -> PathBuf {
    let bundle = work.join("bundle");
    fs::create_dir(&bundle).unwrap();
    let packages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
    symlink(
        fs::canonicalize(packages.join("core")).unwrap(),
        bundle.join("core"),
    )
    .unwrap();
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/nomi_capabilities/fake");
    symlink(fs::canonicalize(fake).unwrap(), bundle.join("fake")).unwrap();
    bundle
}

/// [`PROGRAM`] built into `dist/main.mjs` in a new directory, offline from
/// an empty store and the bundle; the program runs in `run/`, beside it.
fn built() -> (tempfile::TempDir, PathBuf) {
    let work = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(work.path()).unwrap();
    fs::write(
        root.join("elm.json"),
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "1.0.0", "nomi/fake": "1.0.0"}, "indirect": {}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Main.elm"), PROGRAM).unwrap();
    let cache = website::fixture_cache();
    let built = offline(Path::new(env!("CARGO_BIN_EXE_nomi")), cache.path())
        .current_dir(&root)
        .args(["build", "src/Main.elm", "--output=dist/main.mjs"])
        .env("NOMI_BUNDLE", bundle(&root))
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    fs::create_dir(root.join("run")).unwrap();
    (work, root)
}

/// The program in `root` run with `args` in `root/run`, `stdin` its input.
fn run(root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new("node")
        .arg(root.join("dist/main.mjs"))
        .args(args)
        .current_dir(root.join("run"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (child.stdin.take().unwrap())
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// The program's standard output, once it exited 0.
fn printed(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Standard error past the development mode's warning.
fn errors(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    (text.split_inclusive('\n'))
        .filter(|line| !line.starts_with("Compiled in DEV mode."))
        .collect()
}

#[test]
fn the_file_systems_on_disk_and_in_memory_give_the_tables_results() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    fs::create_dir(root.join("run/scratch")).unwrap();
    let real = run(&root, &["table", "real"], "");
    assert_eq!(printed(&real), TABLE);
    assert_eq!(
        fs::read_to_string(root.join("run/scratch/table/a.txt")).unwrap(),
        "hello, world"
    );
    let memory = run(&root, &["table", "memory"], "");
    assert_eq!(printed(&memory), TABLE);
}

#[test]
fn no_symbolic_link_leads_out_of_a_subdir() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    let run_dir = root.join("run");
    fs::create_dir(run_dir.join("outside")).unwrap();
    fs::write(run_dir.join("outside/secret.txt"), "secret").unwrap();
    let links = run_dir.join("scratch/links");
    fs::create_dir_all(&links).unwrap();
    fs::write(links.join("plain.txt"), "plain").unwrap();
    symlink("plain.txt", links.join("alias.txt")).unwrap();
    symlink("../../outside", links.join("escape")).unwrap();
    symlink("../../outside/secret.txt", links.join("secret.txt")).unwrap();
    symlink("../../outside/new.txt", links.join("dangling")).unwrap();
    symlink("..", links.join("up")).unwrap();
    let ran = run(&root, &["links"], "");
    assert_eq!(
        printed(&ran),
        r#"read alias.txt: Ok "plain"
read secret.txt: Err (OutsideRoot "secret.txt")
read escape/secret.txt: Err (OutsideRoot "escape/secret.txt")
list escape: Err (OutsideRoot "escape")
write escape/new.txt: Err (OutsideRoot "escape/new.txt")
write secret.txt: Err (OutsideRoot "secret.txt")
write dangling: Err (OutsideRoot "dangling")
append dangling: Err (OutsideRoot "dangling")
makeDirectory escape/made: Err (OutsideRoot "escape/made")
exists escape/secret.txt: Ok False
subdir escape: read secret.txt: Err (OutsideRoot "secret.txt")
subdir up: list .: Err (OutsideRoot ".")
remove secret.txt: Ok ()
exists secret.txt: Ok False
"#
    );
    let outside: Vec<_> = fs::read_dir(run_dir.join("outside"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(outside, ["secret.txt"]);
    assert_eq!(
        fs::read_to_string(run_dir.join("outside/secret.txt")).unwrap(),
        "secret"
    );
}

#[test]
fn stdio_reads_lines_to_the_end_of_input() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    for (input, expected) in [
        (
            "one\ntwo\r\n\nthree\n",
            "Just \"one\"\nJust \"two\"\nJust \"\"\nJust \"three\"\nNothing\n",
        ),
        ("one\ntwo", "Just \"one\"\nJust \"two\"\nNothing\n"),
        ("", "Nothing\n"),
    ] {
        let ran = run(&root, &["lines"], input);
        assert_eq!(printed(&ran), expected, "{input:?}");
        assert_eq!(errors(&ran), "end", "{input:?}");
    }
    let fake = run(&root, &["fake-stdio"], "");
    assert_eq!(
        printed(&fake),
        "Just \"a\"\nJust \"b\"\nJust \"\"\nJust \"c\"\nNothing\n"
    );
    assert_eq!(errors(&fake), "end");
}

#[test]
fn process_exit_runs_the_finalizers_then_exits_with_its_code() {
    nomi_testkit::require_node();
    let (_work, root) = built();
    let ran = run(&root, &["exit"], "");
    assert_eq!(ran.status.code(), Some(3), "{ran:?}");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "exiting\nchild finalized\nfinalized\n"
    );
    assert_eq!(errors(&ran), "");
}
