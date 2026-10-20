//! nomi/core's capabilities, bundled with nomi: `Nomi.Fs` on disk and in
//! memory gives the same results for one table of cases, and neither `..`
//! nor a symbolic link leads out of a `subdir`; `Nomi.Stdio` reads lines
//! to the end of input as its fake does, refusing a line that isn't
//! UTF-8, and a line read interrupted while input is on its way loses
//! none; a pipe nobody reads holds back writes to stdout;
//! `Nomi.Process.exit` runs finalizers, then exits.
//! The fakes are reached as nomi/test reaches them, from another bundled
//! package's kernel: the fixture nomi/fake, bundled beside nomi/core.
#![cfg(unix)]

#[path = "support/website.rs"]
pub mod website;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use website::offline;

/// A program picked by its arguments: `table real` or `table memory` runs
/// the file system's table on `scratch/` or on a memory file system;
/// `links` the cases only the disk has, in `scratch/links/`; `lines`
/// reads standard input to its end by lines, and `fake-lines` does the
/// same to a fake input holding the bytes standard input held, writing to
/// a fake output; `late` waits 200 ms for a line, then reads by lines;
/// `flood` writes 200 chunks of 64 KiB, saying on stderr as each is
/// taken; `owned` opens `big.bin` and watches `.` in a fiber, waits for
/// a change of that watch in another, says so, and once a line comes
/// interrupts the first, says what the change was and that it's done,
/// then waits for another line; `commands real` and `commands fake`
/// run commands on the system and on a fake answering as it does,
/// the fake saying what it ran; `fake-child` waits for a fake child,
/// which exits once its input is closed; `grace` interrupts the fiber of
/// a child that ignores SIGTERM, once a line comes; `hangup` listens for
/// SIGHUP and SIGTERM and says what came; `bad-files` uses memory file systems given files the disk can't
/// hold; `exit` exits 3 with finalizers to run, and `exit <code>` with the
/// code.
const PROGRAM: &str = r#"module Main exposing (main)

import Bytes
import Bytes.Decode
import Bytes.Encode
import Nomi exposing (Env)
import Nomi.Fake as Fake
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Process
import Nomi.Process as Proc
import Nomi.Queue as Queue
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
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

                [ "fake-lines" ] ->
                    Stream.readAll { limit = 1000000 } env.stdin
                        |> Task.mapError Stream.errorToString
                        |> Task.andThen (\input -> Task.map2 Tuple.pair (Fake.reader input) Fake.writer)
                        |> Task.andThen
                            (\( reader, writer ) ->
                                lines { env | stdin = reader, stdout = writer }
                                    |> Task.andThen (\_ -> Fake.captured writer)
                            )
                        |> Task.andThen (\text -> say text env)

                [ "late" ] ->
                    Nomi.Task.timeout 200 (Stdio.line env.stdin)
                        |> Task.mapError Stream.errorToString
                        |> Task.andThen (\first -> say (Debug.toString first ++ "\n") env)
                        |> Task.andThen (\_ -> lines env)

                [ "flood" ] ->
                    flood env 1

                [ "commands", "real" ] ->
                    commands env env.processes

                [ "commands", "fake" ] ->
                    Proc.fake emulated
                        |> Task.andThen (\processes -> commands env processes |> Task.andThen (\_ -> Proc.ran processes))
                        |> Task.andThen (\ran -> say (String.concat (List.map ranLine ran)) env)

                [ "fake-child" ] ->
                    Proc.fake (\_ -> Just { status = 7, stdout = bytes [], stderr = bytes [] })
                        |> Task.andThen
                            (\processes ->
                                Proc.spawn (Proc.command "cat" []) processes
                                    |> Task.mapError Proc.errorToString
                                    |> Task.andThen
                                        (\child ->
                                            Nomi.Task.timeout 100 (Proc.wait child)
                                                |> Task.mapError Proc.errorToString
                                                |> Task.andThen (\early -> say ("before closing: " ++ Debug.toString early ++ "\n") env)
                                                |> Task.andThen (\_ -> Stream.write (utf8 "abc") (Proc.stdin child) |> Task.andThen (\_ -> Stream.close (Proc.stdin child)) |> Task.mapError Stream.errorToString)
                                                |> Task.andThen (\_ -> Proc.wait child |> Task.mapError Proc.errorToString)
                                                |> Task.andThen (\status -> say ("after closing: " ++ String.fromInt status ++ "\n") env)
                                        )
                                    |> Task.andThen (\_ -> Proc.ran processes)
                                    |> Task.andThen (\ran -> say (String.concat (List.map ranLine ran)) env)
                            )

                [ "grace" ] ->
                    Fiber.fork
                        (Proc.spawn (Proc.command "sh" [ "-c", "trap 'echo term >> term.log' TERM; echo ready; while :; do sleep 0.05; done" ] |> Proc.withGrace 500) env.processes
                            |> Task.mapError Proc.errorToString
                            |> Task.andThen (\child -> Stdio.line (Proc.stdout child) |> Task.mapError Stream.errorToString)
                            |> Task.andThen (\_ -> say "ready\n" env)
                            |> Task.andThen (\_ -> Process.sleep 3600000)
                        )
                        |> Task.andThen (\owner -> next env |> Task.andThen (\_ -> Fiber.interrupt owner))
                        |> Task.andThen (\_ -> say "interrupted\n" env)

                [ "hangup" ] ->
                    Proc.signals [ Proc.Hangup, Proc.Terminate ]
                        |> Task.andThen
                            (\listening ->
                                say "listening\n" env
                                    |> Task.andThen (\_ -> Proc.signal listening)
                                    |> Task.andThen (\got -> say (Debug.toString got ++ "\n") env)
                                    |> Task.andThen (\_ -> Process.sleep 3600000)
                            )

                [ "owned" ] ->
                    Queue.unbounded
                        |> Task.andThen
                            (\watches ->
                                Fiber.fork
                                    (Fs.reader "big.bin" env.fs
                                        |> Task.andThen (\from -> Fs.watch "." env.fs |> Task.andThen (\watch -> Queue.offer watch watches) |> Task.map (\_ -> from))
                                        |> Task.andThen (\from -> Stream.read from |> Task.mapError (\_ -> Fs.Other "read"))
                                        |> Task.andThen (\_ -> Process.sleep 3600000)
                                        |> Task.mapError Fs.errorToString
                                    )
                                    |> Task.andThen (\owner -> Queue.take watches |> Task.map (Tuple.pair owner))
                            )
                        |> Task.andThen
                            (\( owner, watch ) ->
                                Fiber.fork (Fs.change watch |> Task.mapError Fs.errorToString |> Task.andThen (\changed -> say ("change: " ++ Debug.toString changed ++ "\n") env))
                                    |> Task.andThen (\waiting -> say "opened\n" env |> Task.map (\_ -> waiting))
                                    |> Task.andThen (\waiting -> next env |> Task.andThen (\_ -> Fiber.interrupt owner) |> Task.andThen (\_ -> Fiber.join waiting))
                            )
                        |> Task.andThen (\_ -> say "closed\n" env)
                        |> Task.andThen (\_ -> next env)

                [ "bad-files" ] ->
                    [ [ ( "../x", "x" ) ], [ ( "", "x" ) ], [ ( "a", "x" ), ( "a/b", "y" ) ], [ ( "a/b", "y" ), ( "a", "x" ) ] ]
                        |> List.map (\files -> Fake.fs files |> Task.andThen (\fs -> Fiber.fork (Fs.exists "a" fs) |> Task.andThen Fiber.await) |> Task.map Debug.toString)
                        |> Task.sequence
                        |> Task.andThen (\outcomes -> say (String.concat (List.map (\line -> line ++ "\n") outcomes)) env)

                [ "exit", code ] ->
                    Nomi.Process.exit (String.toInt code |> Maybe.withDefault 0)

                [ "exit" ] ->
                    Fiber.fork (Process.sleep 3600000 |> Nomi.Task.ensure (say "child finalized\n" env))
                        |> Task.andThen (\_ -> say "exiting\n" env)
                        |> Task.andThen (\_ -> Nomi.Process.exit 3)
                        |> Task.andThen (\_ -> say "unreachable\n" env)
                        |> Nomi.Task.ensure (say "finalized\n" env)

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
    , outcome "writeBytes dir/sub/c.bin" (Fs.writeBytes "dir/sub/c.bin" (Bytes.Encode.encode (Bytes.Encode.string "héllo")) fs)
    , outcome "read dir/sub/c.bin" (Fs.read "dir/sub/c.bin" fs)
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
    , outcome "subdir new/deeper: makeDirectory inner" (Fs.makeDirectory "inner" (Fs.subdir "new/deeper" fs))
    , outcome "subdir new/deeper: write inner/x.txt" (Fs.write "inner/x.txt" "x" (Fs.subdir "new/deeper" fs))
    , outcome "read new/deeper/inner/x.txt" (Fs.read "new/deeper/inner/x.txt" fs)
    , outcome "subdir more: makeDirectory ." (Fs.makeDirectory "." (Fs.subdir "more" fs))
    , outcome "subdir more: list ." (Fs.list "." (Fs.subdir "more" fs))
    , outcome "remove new" (Fs.remove { recursive = True } "new" fs)
    , outcome "remove more" (Fs.remove { recursive = True } "more" fs)
    , outcome "readOnly: read a.txt" (Fs.read "a.txt" readOnly)
    , outcome "readOnly: write a.txt" (Fs.write "a.txt" "x" readOnly)
    , outcome "readOnly: append a.txt" (Fs.append "a.txt" "x" readOnly)
    , outcome "readOnly: remove a.txt" (Fs.remove { recursive = True } "a.txt" readOnly)
    , outcome "readOnly: makeDirectory d" (Fs.makeDirectory "d" readOnly)
    , outcome "readOnly, subdir dir: write b.txt" (Fs.write "b.txt" "x" (Fs.subdir "dir" readOnly))
    , outcome "readOnly: rename a.txt b.txt" (Fs.rename "a.txt" "b.txt" readOnly)
    , outcome "readOnly: copy a.txt b.txt" (Fs.copy "a.txt" "b.txt" readOnly)
    , outcome "readOnly: symlink a.txt l" (Fs.symlink "a.txt" "l" readOnly)
    , outcome "readOnly: writer a.txt" (Fs.writer "a.txt" readOnly |> Task.map (\_ -> ()))
    , outcome "readOnly: temporaryDirectory" (Fs.temporaryDirectory readOnly |> Task.map (\_ -> ()))
    , outcome "remove dir, not recursive" (Fs.remove { recursive = False } "dir" fs)
    , outcome "remove dir/sub/b.txt, not recursive" (Fs.remove { recursive = False } "dir/sub/b.txt" fs)
    , outcome "writeBytes bad.txt" (Fs.writeBytes "bad.txt" (bytes [ 0x61, 0xFF ]) fs)
    , outcome "read bad.txt" (Fs.read "bad.txt" fs)
    , outcome "readBytes bad.txt" (Fs.readBytes "bad.txt" fs |> Task.map Bytes.width)
    , outcome "remove bad.txt, not recursive" (Fs.remove { recursive = False } "bad.txt" fs)
    , outcome "stat a.txt" (Fs.stat "a.txt" fs |> Task.map (\s -> ( s.kind, s.size )))
    , outcome "stat dir" (Fs.stat "dir" fs |> Task.map (\s -> ( s.kind, s.size )))
    , outcome "stat missing" (Fs.stat "missing" fs |> Task.map .kind)
    , outcome "rename a.txt moved.txt" (Fs.rename "a.txt" "moved.txt" fs)
    , outcome "read moved.txt" (Fs.read "moved.txt" fs)
    , outcome "exists a.txt" (Fs.exists "a.txt" fs)
    , outcome "rename moved.txt a.txt" (Fs.rename "moved.txt" "a.txt" fs)
    , outcome "rename missing x" (Fs.rename "missing" "x" fs)
    , outcome "rename a.txt dir" (Fs.rename "a.txt" "dir" fs)
    , outcome "rename dir a.txt" (Fs.rename "dir" "a.txt" fs)
    , outcome "rename dir dir/sub/inner" (Fs.rename "dir" "dir/sub/inner" fs)
    , outcome "rename a.txt nope/a.txt" (Fs.rename "a.txt" "nope/a.txt" fs)
    , outcome "rename a.txt ." (Fs.rename "a.txt" "." fs)
    , outcome "write over.txt" (Fs.write "over.txt" "over" fs)
    , outcome "rename over.txt dir/sub/b.txt" (Fs.rename "over.txt" "dir/sub/b.txt" fs)
    , outcome "read dir/sub/b.txt" (Fs.read "dir/sub/b.txt" fs)
    , outcome "copy dir copied" (Fs.copy "dir" "copied" fs)
    , outcome "list copied/sub" (Fs.list "copied/sub" fs)
    , outcome "read copied/sub/b.txt" (Fs.read "copied/sub/b.txt" fs)
    , outcome "copy dir copied" (Fs.copy "dir" "copied" fs)
    , outcome "copy a.txt dir/sub" (Fs.copy "a.txt" "dir/sub" fs)
    , outcome "copy dir dir/sub/inside" (Fs.copy "dir" "dir/sub/inside" fs)
    , outcome "copy missing x" (Fs.copy "missing" "x" fs)
    , outcome "copy a.txt nope/x" (Fs.copy "a.txt" "nope/x" fs)
    , outcome "remove copied" (Fs.remove { recursive = True } "copied" fs)
    , outcome "symlink a.txt link.txt" (Fs.symlink "a.txt" "link.txt" fs)
    , outcome "read link.txt" (Fs.read "link.txt" fs)
    , outcome "readLink link.txt" (Fs.readLink "link.txt" fs)
    , outcome "realPath link.txt" (Fs.realPath "link.txt" fs)
    , outcome "stat link.txt" (Fs.stat "link.txt" fs |> Task.map .kind)
    , outcome "symlink a.txt link.txt" (Fs.symlink "a.txt" "link.txt" fs)
    , outcome "symlink ../../x out" (Fs.symlink "../../x" "out" fs)
    , outcome "symlink /etc/passwd abs" (Fs.symlink "/etc/passwd" "abs" fs)
    , outcome "subdir dir: symlink ../a.txt up" (Fs.symlink "../a.txt" "up" (Fs.subdir "dir" fs))
    , outcome "symlink ../a.txt dir/up" (Fs.symlink "../a.txt" "dir/up" fs)
    , outcome "read dir/up" (Fs.read "dir/up" fs)
    , outcome "subdir dir: read up" (Fs.read "up" (Fs.subdir "dir" fs))
    , outcome "symlink nowhere dangling" (Fs.symlink "nowhere" "dangling" fs)
    , outcome "read dangling" (Fs.read "dangling" fs)
    , outcome "exists dangling" (Fs.exists "dangling" fs)
    , outcome "readLink dangling" (Fs.readLink "dangling" fs)
    , outcome "symlink sub dir/sublink" (Fs.symlink "sub" "dir/sublink" fs)
    , outcome "list dir/sublink" (Fs.list "dir/sublink" fs)
    , outcome "realPath dir/sublink/../sublink/b.txt" (Fs.realPath "dir/sublink/../sublink/b.txt" fs)
    , outcome "realPath ." (Fs.realPath "." fs)
    , outcome "realPath missing" (Fs.realPath "missing" fs)
    , outcome "readLink a.txt" (Fs.readLink "a.txt" fs)
    , outcome "rename link.txt link2.txt" (Fs.rename "link.txt" "link2.txt" fs)
    , outcome "readLink link2.txt" (Fs.readLink "link2.txt" fs)
    , outcome "remove link2.txt" (Fs.remove { recursive = False } "link2.txt" fs)
    , outcome "exists a.txt" (Fs.exists "a.txt" fs)
    , outcome "remove dangling" (Fs.remove { recursive = False } "dangling" fs)
    , outcome "remove dir/up" (Fs.remove { recursive = False } "dir/up" fs)
    , outcome "remove dir/sublink" (Fs.remove { recursive = False } "dir/sublink" fs)
    , outcome "stream w.txt" (streamed fs)
    , outcome "reader missing" (Fs.reader "missing" fs |> Task.map (\_ -> ()))
    , outcome "reader dir" (Fs.reader "dir" fs |> Task.andThen (\from -> Stream.read from |> Task.mapError (\_ -> Fs.Other "read")) |> Task.map (\_ -> ()))
    , outcome "writer dir" (Fs.writer "dir" fs |> Task.map (\_ -> ()))
    , outcome "remove w.txt" (Fs.remove { recursive = False } "w.txt" fs)
    , outcome "temporaryDirectory" (temporary fs)
    , outcome "read a.txt" (Fs.read "a.txt" fs)
    , outcome "remove ." (Fs.remove { recursive = True } "." fs)
    , outcome "remove dir" (Fs.remove { recursive = True } "dir" fs)
    , outcome "exists dir/sub/b.txt" (Fs.exists "dir/sub/b.txt" fs)
    , outcome "remove dir" (Fs.remove { recursive = True } "dir" fs)
    , outcome "remove new.txt" (Fs.remove { recursive = True } "new.txt" fs)
    , outcome "list ." (Fs.list "." fs)
    ]


bytes : List Int -> Bytes.Bytes
bytes ints =
    Bytes.Encode.encode (Bytes.Encode.sequence (List.map Bytes.Encode.unsignedInt8 ints))


{-| A file written, appended to and read as streams.
-}
streamed : Fs -> Task String String
streamed fs =
    let
        text =
            String.join "" >> Bytes.Encode.string >> Bytes.Encode.encode

        put open chunks =
            open "w.txt" fs
                |> Task.mapError Fs.errorToString
                |> Task.andThen
                    (\to ->
                        Task.sequence (List.map (\chunk -> Stream.write (text [ chunk ]) to) chunks)
                            |> Task.andThen (\_ -> Stream.close to)
                            |> Task.andThen (\_ -> Stream.write (text [ "late" ]) to |> Task.map (\_ -> False) |> Task.onError (\_ -> Task.succeed True))
                            |> Task.mapError Stream.errorToString
                            |> Task.andThen
                                (\refused ->
                                    if refused then
                                        Task.succeed ()

                                    else
                                        Task.fail "written after close"
                                )
                    )
    in
    put Fs.writer [ "ab", "c" ]
        |> Task.andThen (\_ -> put Fs.appender [ "d" ])
        |> Task.andThen (\_ -> Fs.reader "w.txt" fs |> Task.mapError Fs.errorToString)
        |> Task.andThen (Stream.readAll { limit = 100 } >> Task.mapError Stream.errorToString)
        |> Task.map (\all -> Bytes.Decode.decode (Bytes.Decode.string (Bytes.width all)) all |> Maybe.withDefault "?")


{-| A temporary directory written to and listed, and whether the
capability it was made from sees it.
-}
temporary : Fs -> Task Fs.Error ( List String, Bool )
temporary fs =
    Fs.temporaryDirectory fs
        |> Task.andThen
            (\scratch ->
                Fs.write "t.txt" "t" scratch
                    |> Task.andThen (\_ -> Fs.list "." scratch)
                    |> Task.andThen (\listed -> Fs.exists "t.txt" fs |> Task.map (Tuple.pair listed))
            )


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
    , outcome "stat secret.txt" (Fs.stat "secret.txt" fs |> Task.map .kind)
    , outcome "realPath escape" (Fs.realPath "escape" fs)
    , outcome "readLink secret.txt" (Fs.readLink "secret.txt" fs)
    , outcome "copy escape/secret.txt x" (Fs.copy "escape/secret.txt" "x" fs)
    , outcome "copy plain.txt escape/x" (Fs.copy "plain.txt" "escape/x" fs)
    , outcome "rename plain.txt escape/x" (Fs.rename "plain.txt" "escape/x" fs)
    , outcome "symlink plain.txt escape/l" (Fs.symlink "plain.txt" "escape/l" fs)
    , outcome "reader secret.txt" (Fs.reader "secret.txt" fs |> Task.map (\_ -> ()))
    , outcome "writer escape/new.txt" (Fs.writer "escape/new.txt" fs |> Task.map (\_ -> ()))
    , outcome "watch escape" (Fs.watch "escape" fs |> Task.map (\_ -> ()))
    , outcome "remove secret.txt" (Fs.remove { recursive = True } "secret.txt" fs)
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
        |> Task.andThen (\outcomes -> say (String.concat (List.map (\line -> line ++ "\n") outcomes)) env)


{-| Commands run on `processes`, real or fake, and what each gave.
-}
commands : Env -> Proc.Processes -> Task String ()
commands env processes =
    let
        shown result =
            case result of
                Ok out ->
                    "Ok " ++ String.fromInt out.status ++ " " ++ Debug.toString (decoded out.stdout) ++ " " ++ Debug.toString (decoded out.stderr)

                Err error ->
                    "Err " ++ Debug.toString error

        run name cmd =
            Proc.output cmd processes
                |> Task.map Ok
                |> Task.onError (Err >> Task.succeed)
                |> Task.map (\result -> name ++ ": " ++ shown result ++ "\n")

        spawned =
            Proc.spawn (Proc.command "printf" [ "%s\n", "spawned" ]) processes
                |> Task.mapError Proc.errorToString
                |> Task.andThen
                    (\child ->
                        Stream.close (Proc.stdin child)
                            |> Task.andThen (\_ -> Stream.readAll { limit = 1000 } (Proc.stdout child))
                            |> Task.mapError Stream.errorToString
                            |> Task.andThen (\out -> Proc.wait child |> Task.mapError Proc.errorToString |> Task.map (\status -> "spawn printf: " ++ String.fromInt status ++ " " ++ Debug.toString (decoded out) ++ "\n"))
                    )
    in
    Task.sequence
        [ run "printf" (Proc.command "printf" [ "%s\n", "a b", "", "c'd", "$HOME", "*" ])
        , run "cat" (Proc.command "cat" [] |> Proc.withStdin (utf8 "fed"))
        , run "sh" (Proc.command "sh" [ "-c", "echo err >&2; exit 3" ])
        , run "missing" (Proc.command "no-such-program" [])
        , spawned
        ]
        |> Task.andThen (\outcomes -> say (String.concat outcomes) env)


{-| What the commands `commands` runs give, as a fake answers them.
-}
emulated : Proc.Command -> Maybe Proc.Output
emulated cmd =
    case ( Proc.program cmd, Proc.arguments cmd ) of
        ( "printf", "%s\n" :: args ) ->
            Just { status = 0, stdout = utf8 (String.concat (List.map (\arg -> arg ++ "\n") args)), stderr = bytes [] }

        ( "cat", [] ) ->
            Just { status = 0, stdout = Proc.input cmd, stderr = bytes [] }

        ( "sh", [ "-c", "echo err >&2; exit 3" ] ) ->
            Just { status = 3, stdout = bytes [], stderr = utf8 "err\n" }

        _ ->
            Nothing


ranLine : { command : Proc.Command, stdin : Bytes.Bytes } -> String
ranLine { command, stdin } =
    "ran " ++ Debug.toString ( Proc.program command, Proc.arguments command, decoded stdin ) ++ "\n"


utf8 : String -> Bytes.Bytes
utf8 =
    Bytes.Encode.string >> Bytes.Encode.encode


decoded : Bytes.Bytes -> String
decoded all =
    Bytes.Decode.decode (Bytes.Decode.string (Bytes.width all)) all |> Maybe.withDefault "?"


{-| Waits for a line of input.
-}
next : Env -> Task String ()
next env =
    Stdio.line env.stdin |> Task.mapError Stream.errorToString |> Task.map (\_ -> ())


{-| Writes to stdout, which never fails.
-}
say : String -> Env -> Task x ()
say text env =
    Stdio.print text env.stdout |> Task.onError (\_ -> Task.succeed ())


lines : Env -> Task x ()
lines env =
    Stdio.line env.stdin
        |> Task.map Ok
        |> Task.onError (Err >> Task.succeed)
        |> Task.andThen
            (\line ->
                say (Debug.toString line ++ "\n") env
                    |> Task.andThen
                        (\_ ->
                            case line of
                                Ok (Just _) ->
                                    lines env

                                _ ->
                                    Stdio.print "end" env.stderr |> Task.onError (\_ -> Task.succeed ())
                        )
            )


flood : Env -> Int -> Task x ()
flood env n =
    if n > 200 then
        Task.succeed ()

    else
        Stream.write (Bytes.Encode.encode (Bytes.Encode.sequence (List.repeat 16384 (Bytes.Encode.unsignedInt32 Bytes.BE 0)))) env.stdout
            |> Task.andThen (\_ -> Stdio.printLine ("wrote " ++ String.fromInt n) env.stderr)
            |> Task.onError (\_ -> Task.succeed ())
            |> Task.andThen (\_ -> flood env (n + 1))
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
writeBytes dir/sub/c.bin: Ok ()
read dir/sub/c.bin: Ok "héllo"
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
subdir new/deeper: makeDirectory inner: Ok ()
subdir new/deeper: write inner/x.txt: Ok ()
read new/deeper/inner/x.txt: Ok "x"
subdir more: makeDirectory .: Ok ()
subdir more: list .: Ok []
remove new: Ok ()
remove more: Ok ()
readOnly: read a.txt: Ok "hello, world"
readOnly: write a.txt: Err (PermissionDenied "a.txt")
readOnly: append a.txt: Err (PermissionDenied "a.txt")
readOnly: remove a.txt: Err (PermissionDenied "a.txt")
readOnly: makeDirectory d: Err (PermissionDenied "d")
readOnly, subdir dir: write b.txt: Err (PermissionDenied "b.txt")
readOnly: rename a.txt b.txt: Err (PermissionDenied "a.txt")
readOnly: copy a.txt b.txt: Err (PermissionDenied "a.txt")
readOnly: symlink a.txt l: Err (PermissionDenied "l")
readOnly: writer a.txt: Err (PermissionDenied "a.txt")
readOnly: temporaryDirectory: Err (PermissionDenied "temporary directory")
remove dir, not recursive: Err (NotEmpty "dir")
remove dir/sub/b.txt, not recursive: Ok ()
writeBytes bad.txt: Ok ()
read bad.txt: Err (NotUtf8 "bad.txt")
readBytes bad.txt: Ok 2
remove bad.txt, not recursive: Ok ()
stat a.txt: Ok (File,12)
stat dir: Ok (Directory,0)
stat missing: Err (NotFound "missing")
rename a.txt moved.txt: Ok ()
read moved.txt: Ok "hello, world"
exists a.txt: Ok False
rename moved.txt a.txt: Ok ()
rename missing x: Err (NotFound "missing")
rename a.txt dir: Err (IsADirectory "dir")
rename dir a.txt: Err (Other "a.txt: not a directory")
rename dir dir/sub/inner: Err (Other "dir: would go inside itself")
rename a.txt nope/a.txt: Err (NotFound "nope/a.txt")
rename a.txt .: Err (PermissionDenied ".")
write over.txt: Ok ()
rename over.txt dir/sub/b.txt: Ok ()
read dir/sub/b.txt: Ok "over"
copy dir copied: Ok ()
list copied/sub: Ok ["b.txt","c.bin"]
read copied/sub/b.txt: Ok "over"
copy dir copied: Err (AlreadyExists "copied")
copy a.txt dir/sub: Err (AlreadyExists "dir/sub")
copy dir dir/sub/inside: Err (Other "dir: would go inside itself")
copy missing x: Err (NotFound "missing")
copy a.txt nope/x: Err (NotFound "nope/x")
remove copied: Ok ()
symlink a.txt link.txt: Ok ()
read link.txt: Ok "hello, world"
readLink link.txt: Ok "a.txt"
realPath link.txt: Ok "a.txt"
stat link.txt: Ok File
symlink a.txt link.txt: Err (AlreadyExists "link.txt")
symlink ../../x out: Err (OutsideRoot "out")
symlink /etc/passwd abs: Err (OutsideRoot "abs")
subdir dir: symlink ../a.txt up: Err (OutsideRoot "up")
symlink ../a.txt dir/up: Ok ()
read dir/up: Ok "hello, world"
subdir dir: read up: Err (OutsideRoot "up")
symlink nowhere dangling: Ok ()
read dangling: Err (OutsideRoot "dangling")
exists dangling: Ok False
readLink dangling: Ok "nowhere"
symlink sub dir/sublink: Ok ()
list dir/sublink: Ok ["b.txt","c.bin"]
realPath dir/sublink/../sublink/b.txt: Ok "dir/sub/b.txt"
realPath .: Ok "."
realPath missing: Err (NotFound "missing")
readLink a.txt: Err (Other "a.txt: not a symbolic link")
rename link.txt link2.txt: Ok ()
readLink link2.txt: Ok "a.txt"
remove link2.txt: Ok ()
exists a.txt: Ok True
remove dangling: Ok ()
remove dir/up: Ok ()
remove dir/sublink: Ok ()
stream w.txt: Ok "abcd"
reader missing: Err (NotFound "missing")
reader dir: Err (IsADirectory "dir")
writer dir: Err (IsADirectory "dir")
remove w.txt: Ok ()
temporaryDirectory: Ok (["t.txt"],False)
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
        r#"{"type": "application", "source-directories": ["src"], "elm-version": "0.19.1", "dependencies": {"direct": {"elm/bytes": "1.0.8", "elm/core": "1.0.5", "elm/json": "1.1.4", "nomi/core": "2.0.0", "nomi/fake": "1.0.0"}, "indirect": {"elm/time": "1.0.0"}}, "test-dependencies": {"direct": {}, "indirect": {}}}"#,
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

/// The program in `root` started with `args` in `root/run`, its standard
/// streams piped.
fn start(root: &Path, args: &[&str]) -> Child {
    Command::new(nomi_testkit::bun())
        .arg(root.join("dist/main.mjs"))
        .args(args)
        .current_dir(root.join("run"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The program in `root` run with `args` in `root/run`, `stdin` its input.
fn run(root: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = start(root, args);
    child.stdin.take().unwrap().write_all(stdin).unwrap();
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
    nomi_testkit::require_bun();
    let (_work, root) = built();
    fs::create_dir(root.join("run/scratch")).unwrap();
    let real = printed(&run(&root, &["table", "real"], b""));
    let memory = printed(&run(&root, &["table", "memory"], b""));
    let differ: Vec<_> = (real.lines().zip(memory.lines()))
        .filter(|(disk, memory)| disk != memory)
        .collect();
    assert!(differ.is_empty(), "disk, then memory: {differ:#?}");
    assert_eq!(real, TABLE);
    assert_eq!(memory, TABLE);
    assert_eq!(
        fs::read_to_string(root.join("run/scratch/table/a.txt")).unwrap(),
        "hello, world"
    );
}

#[test]
fn no_symbolic_link_leads_out_of_a_subdir() {
    nomi_testkit::require_bun();
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
    let ran = run(&root, &["links"], b"");
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
stat secret.txt: Err (OutsideRoot "secret.txt")
realPath escape: Err (OutsideRoot "escape")
readLink secret.txt: Ok "../../outside/secret.txt"
copy escape/secret.txt x: Err (OutsideRoot "escape/secret.txt")
copy plain.txt escape/x: Err (OutsideRoot "escape/x")
rename plain.txt escape/x: Err (OutsideRoot "escape/x")
symlink plain.txt escape/l: Err (OutsideRoot "escape/l")
reader secret.txt: Err (OutsideRoot "secret.txt")
writer escape/new.txt: Err (OutsideRoot "escape/new.txt")
watch escape: Err (OutsideRoot "escape")
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
fn stdio_reads_lines_to_the_end_of_input_as_its_fake_does() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    for (input, expected) in [
        (
            &b"one\ntwo\r\n\nthree\n"[..],
            "Ok (Just \"one\")\nOk (Just \"two\")\nOk (Just \"\")\nOk (Just \"three\")\nOk Nothing\n",
        ),
        (
            b"one\ntwo",
            "Ok (Just \"one\")\nOk (Just \"two\")\nOk Nothing\n",
        ),
        (b"", "Ok Nothing\n"),
        (
            b"ok\n\xff\xfe\nafter\n",
            "Ok (Just \"ok\")\nErr (Invalid \"a line is not UTF-8\")\n",
        ),
    ] {
        for mode in ["lines", "fake-lines"] {
            let ran = run(&root, &[mode], input);
            assert_eq!(printed(&ran), expected, "{mode} {input:?}");
            assert_eq!(errors(&ran), "end", "{mode} {input:?}");
        }
    }
}

#[test]
fn a_line_read_interrupted_while_input_is_on_its_way_loses_none() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let mut late = start(&root, &["late"]);
    let mut input = late.stdin.take().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    input.write_all(b"first\nsecond\n").unwrap();
    drop(input);
    let ran = late.wait_with_output().unwrap();
    assert_eq!(
        printed(&ran),
        "Nothing\nOk (Just \"first\")\nOk (Just \"second\")\nOk Nothing\n"
    );
}

#[test]
fn a_pipe_nobody_reads_holds_back_writes_to_stdout() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let mut flood = start(&root, &["flood"]);
    let said = BufReader::new(flood.stderr.take().unwrap());
    let last = Arc::new(AtomicUsize::new(0));
    let counting = {
        let last = Arc::clone(&last);
        std::thread::spawn(move || {
            for line in said.lines() {
                if let Some(n) = line.unwrap().strip_prefix("wrote ") {
                    last.store(n.parse().unwrap(), Ordering::SeqCst);
                }
            }
        })
    };
    // 200 chunks of 64 KiB are 12.5 MiB; a pipe holds 64 KiB.
    std::thread::sleep(Duration::from_millis(500));
    let held = last.load(Ordering::SeqCst);
    assert!(held < 20, "{held} chunks written to a pipe nobody reads");
    let mut out = Vec::new();
    flood.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    assert_eq!(out.len(), 200 * 64 * 1024);
    assert!(flood.wait().unwrap().success());
    counting.join().unwrap();
    assert_eq!(last.load(Ordering::SeqCst), 200);
}

#[test]
fn process_exit_runs_the_finalizers_then_exits_with_its_code() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let ran = run(&root, &["exit"], b"");
    assert_eq!(ran.status.code(), Some(3), "{ran:?}");
    assert_eq!(
        String::from_utf8_lossy(&ran.stdout),
        "exiting\nchild finalized\nfinalized\n"
    );
    assert_eq!(errors(&ran), "");
}

#[test]
fn a_memory_file_system_given_files_the_disk_cant_hold_is_a_defect_to_use() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let ran = run(&root, &["bad-files"], b"");
    assert_eq!(
        printed(&ran),
        r#"Defect "Error: withFiles: \"../x\" is outside the root"
Defect "Error: withFiles: \"\" names no file"
Defect "Error: withFiles: \"a/b\" is under a file"
Defect "Error: withFiles: \"a\" is a directory"
"#
    );
}

#[test]
fn process_exit_with_a_code_out_of_range_exits_255() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    for (code, exited) in [("0", 0), ("255", 255), ("256", 255), ("-1", 255)] {
        let ran = run(&root, &["exit", code], b"");
        assert_eq!(ran.status.code(), Some(exited), "{code}: {ran:?}");
    }
}

/// The open files of process `pid`: what each descriptor leads to.
fn descriptors(pid: u32) -> Vec<String> {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| fs::read_link(entry.unwrap().path()).ok())
        .map(|target| target.to_string_lossy().into_owned())
        .collect()
}

#[test]
#[cfg(target_os = "linux")]
fn a_file_and_a_watch_close_once_the_fiber_that_opened_them_is_interrupted() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    fs::write(root.join("run/big.bin"), vec![0; 1 << 20]).unwrap();
    let mut owned = start(&root, &["owned"]);
    let pid = owned.id();
    let mut input = owned.stdin.take().unwrap();
    let mut said = BufReader::new(owned.stdout.take().unwrap()).lines();
    assert_eq!(said.next().unwrap().unwrap(), "opened");
    let open = |all: &[String]| {
        all.iter()
            .filter(|target| target.ends_with("/big.bin"))
            .count()
    };
    let before = descriptors(pid);
    assert_eq!(open(&before), 1, "{before:?}");
    input.write_all(b"go\n").unwrap();
    // Bun keeps one inotify descriptor for every watch, so the watch
    // stopping shows as the fiber waiting on it told so.
    assert_eq!(said.next().unwrap().unwrap(), "change: Nothing");
    assert_eq!(said.next().unwrap().unwrap(), "closed");
    let after = descriptors(pid);
    assert_eq!(open(&after), 0, "{after:?}");
    input.write_all(b"end\n").unwrap();
    assert!(owned.wait().unwrap().success());
}

#[test]
fn commands_give_the_same_results_on_a_fake_which_says_what_it_ran() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let real = printed(&run(&root, &["commands", "real"], b""));
    assert_eq!(
        real,
        r#"printf: Ok 0 "a b\n\nc'd\n$HOME\n*\n" ""
cat: Ok 0 "fed" ""
sh: Ok 3 "" "err\n"
missing: Err NotFound "no-such-program"
spawn printf: 0 "spawned\n"
"#
    );
    let fake = printed(&run(&root, &["commands", "fake"], b""));
    assert_eq!(
        fake,
        real + r#"ran ("printf",["%s\n","a b","","c'd","$HOME","*"],"")
ran ("cat",[],"fed")
ran ("sh",["-c","echo err >&2; exit 3"],"")
ran ("printf",["%s\n","spawned"],"")
"#
    );
}

#[test]
fn a_fake_child_exits_once_its_input_is_closed() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let ran = run(&root, &["fake-child"], b"");
    assert_eq!(
        printed(&ran),
        "before closing: Nothing\nafter closing: 7\nran (\"cat\",[],\"abc\")\n"
    );
}

#[test]
#[cfg(unix)]
fn an_interrupted_fibers_child_gets_sigterm_then_sigkill_after_its_grace() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let mut grace = start(&root, &["grace"]);
    let mut input = grace.stdin.take().unwrap();
    let mut said = BufReader::new(grace.stdout.take().unwrap()).lines();
    assert_eq!(said.next().unwrap().unwrap(), "ready");
    let children = Command::new("pgrep")
        .args(["-P", &grace.id().to_string()])
        .output()
        .unwrap();
    let child = String::from_utf8_lossy(&children.stdout).trim().to_owned();
    assert!(!child.is_empty(), "no child");
    let interrupted = std::time::Instant::now();
    input.write_all(b"go\n").unwrap();
    assert_eq!(said.next().unwrap().unwrap(), "interrupted");
    let took = interrupted.elapsed();
    // It ignored SIGTERM, so it lived out the 500 ms grace.
    assert!(took >= Duration::from_millis(450), "{took:?}");
    assert!(took < Duration::from_secs(4), "{took:?}");
    assert_eq!(
        fs::read_to_string(root.join("run/term.log")).unwrap(),
        "term\n"
    );
    let gone = Command::new("kill")
        .args(["-0", &child])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!gone.success(), "child {child} still runs");
    drop(input);
    assert!(grace.wait().unwrap().success());
}

#[test]
#[cfg(unix)]
fn sighup_arrives_through_signals_and_sigterm_still_ends_the_program() {
    nomi_testkit::require_bun();
    let (_work, root) = built();
    let mut hangup = start(&root, &["hangup"]);
    let mut said = BufReader::new(hangup.stdout.take().unwrap()).lines();
    assert_eq!(said.next().unwrap().unwrap(), "listening");
    let pid = hangup.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-HUP", &pid])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(said.next().unwrap().unwrap(), "Just Hangup");
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(hangup.wait().unwrap().code(), Some(143));
}
