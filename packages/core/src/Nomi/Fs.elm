module Nomi.Fs exposing
    ( Fs
    , read, readBytes, write, writeBytes, append, list, remove, makeDirectory, exists
    , stat, Stat, Kind(..), rename, copy
    , symlink, readLink, realPath
    , reader, writer, appender
    , Watch, watch, change
    , subdir, readOnly, temporaryDirectory
    , Error(..), errorToString
    )

{-| Files and directories, through a capability: a program reaches only
what the `Fs` it is handed holds, which is the directory it started in
for `Nomi.Env`'s `fs`.

Paths are `/`-separated, on every platform, and relative to the
capability's root: `"data/todos.json"`, where `.` and empty names stay put
and `..` goes up, but never above the root. A path that would is
`OutsideRoot`, and so is one whose symbolic links lead out of it.

@docs Fs


# Files and directories

@docs read, readBytes, write, writeBytes, append, list, remove, makeDirectory, exists
@docs stat, Stat, Kind, rename, copy


# Symbolic links

@docs symlink, readLink, realPath


# Streams

A file read or written chunk by chunk, as a `Nomi.Stream.Reader` or
`Nomi.Stream.Writer`. The file is the fiber's that opened it (as every
handle is): it is closed once that fiber ends, however it ends, if it
wasn't before, at the reader's end or by `Nomi.Stream.close`. Opening
fails with an `Error`; once open, the stream fails with a
`Nomi.Stream.Error`.

@docs reader, writer, appender


# Watching

@docs Watch, watch, change


# Narrowing

@docs subdir, readOnly, temporaryDirectory


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiFs
import Nomi.Stream as Stream
import Task exposing (Task)
import Time


{-| A directory, and everything under it, that a program may read and,
unless it is `readOnly`, write.
-}
type Fs
    = Fs


{-| What went wrong, with the path the operation was given.
-}
type Error
    = NotFound String
    | PermissionDenied String
    | AlreadyExists String
    | IsADirectory String
    | NotEmpty String
    | NotUtf8 String
    | OutsideRoot String
    | Other String


{-| The error as one line: the path, then what went wrong with it.
-}
errorToString : Error -> String
errorToString error =
    case error of
        NotFound path ->
            path ++ ": no such file or directory"

        PermissionDenied path ->
            path ++ ": permission denied"

        AlreadyExists path ->
            path ++ ": already exists"

        IsADirectory path ->
            path ++ ": is a directory"

        NotEmpty path ->
            path ++ ": directory not empty"

        NotUtf8 path ->
            path ++ ": not UTF-8 text"

        OutsideRoot path ->
            path ++ ": outside the root"

        Other message ->
            message


{-| The file's text, decoded as UTF-8. A file that isn't UTF-8 fails with
`NotUtf8`, rather than read with its bad bytes replaced; `readBytes`
reads any file.

    Nomi.Fs.read "todos.json" env.fs

-}
read : String -> Fs -> Task Error String
read =
    Elm.Kernel.NomiFs.read


{-| The file's bytes.
-}
readBytes : String -> Fs -> Task Error Bytes
readBytes =
    Elm.Kernel.NomiFs.readBytes


{-| Writes the text to the file, encoded as UTF-8, creating the file or
replacing what it held. The directory it is in must exist, or the task
fails with `NotFound`; `makeDirectory` makes it.
-}
write : String -> String -> Fs -> Task Error ()
write =
    Elm.Kernel.NomiFs.write


{-| Writes the bytes to the file, as `write` writes text.
-}
writeBytes : String -> Bytes -> Fs -> Task Error ()
writeBytes =
    Elm.Kernel.NomiFs.writeBytes


{-| Adds the text to the end of the file, creating it if there is none.
-}
append : String -> String -> Fs -> Task Error ()
append =
    Elm.Kernel.NomiFs.append


{-| The names in the directory, sorted.
-}
list : String -> Fs -> Task Error (List String)
list =
    Elm.Kernel.NomiFs.list


{-| Removes the file, or the directory: with everything in it when
`recursive`, and otherwise only an empty one, a directory holding anything
failing with `NotEmpty`. A symbolic link is removed, not what it leads
to; the root is never removed (`PermissionDenied`).

    Nomi.Fs.remove { recursive = True } "build" env.fs

-}
remove : { recursive : Bool } -> String -> Fs -> Task Error ()
remove =
    Elm.Kernel.NomiFs.remove


{-| Creates the directory, and those it is in that don't exist yet. A
directory already there is left as it is, so the task succeeds; a file
there fails it with `AlreadyExists`.

    Nomi.Fs.makeDirectory "data" env.fs
        |> Task.andThen (\_ -> Nomi.Fs.write "data/todos.json" "{}" env.fs)

-}
makeDirectory : String -> Fs -> Task Error ()
makeDirectory =
    Elm.Kernel.NomiFs.makeDirectory


{-| Whether there is a file or a directory at the path. It never fails: a
path it can't reach, or one outside the root, is `False`.
-}
exists : String -> Fs -> Task x Bool
exists =
    Elm.Kernel.NomiFs.exists


{-| What a file or a directory is: its kind, its size in bytes (0 for a
directory), and when its contents last changed.
-}
type alias Stat =
    { kind : Kind
    , size : Int
    , modified : Time.Posix
    }


{-| A file, a directory, or something else the system has, as a device or
a socket.
-}
type Kind
    = File
    | Directory
    | Special


{-| What the path leads to, following a symbolic link to what it leads to.
In memory, a file's time is the program's clock when it was written, so
under `nomi test` it is the test's.
-}
stat : String -> Fs -> Task Error Stat
stat =
    Elm.Kernel.NomiFs.stat


{-| Moves the file or directory at the first path to the second, replacing
a file there or an empty directory, as the system's `rename` does. An
error names the path it is about: `NotEmpty` and `IsADirectory` the
second.

    Nomi.Fs.write "todos.json.new" json env.fs
        |> Task.andThen (\_ -> Nomi.Fs.rename "todos.json.new" "todos.json" env.fs)

-}
rename : String -> String -> Fs -> Task Error ()
rename =
    Elm.Kernel.NomiFs.rename


{-| Copies the file, or the directory with everything in it, to the second
path, where nothing may be yet (`AlreadyExists`); the directory it goes
in must exist. A symbolic link inside a directory is copied as a link,
leading where it led.
-}
copy : String -> String -> Fs -> Task Error ()
copy =
    Elm.Kernel.NomiFs.copyPath


{-| Makes a symbolic link at the second path, leading to the first: a path
from the link's directory, which mustn't lead out of the capability
(`OutsideRoot`), nor be absolute.

    Nomi.Fs.symlink "releases/1.2.0" "current" env.fs

-}
symlink : String -> String -> Fs -> Task Error ()
symlink =
    Elm.Kernel.NomiFs.symlink


{-| Where the symbolic link at the path leads, as it was made. A path that
isn't a link fails with `Other`.
-}
readLink : String -> Fs -> Task Error String
readLink =
    Elm.Kernel.NomiFs.readLink


{-| The path, from the capability's root, of what the path leads to once
every symbolic link in it is followed: `"."` for the root. It never shows
what is beyond the root.
-}
realPath : String -> Fs -> Task Error String
realPath =
    Elm.Kernel.NomiFs.realPath


{-| The file's bytes, as a reader of chunks.

    header : Fs -> Task String (Maybe String)
    header fs =
        Nomi.Fs.reader "big.csv" fs
            |> Task.mapError Nomi.Fs.errorToString
            |> Task.andThen Nomi.Stream.lines
            |> Task.andThen (Nomi.Stream.line >> Task.mapError Nomi.Stream.errorToString)

-}
reader : String -> Fs -> Task Error Stream.Reader
reader =
    Elm.Kernel.NomiFs.reader


{-| A writer to the file, which it creates, or empties if it is there.
-}
writer : String -> Fs -> Task Error Stream.Writer
writer =
    Elm.Kernel.NomiFs.writer


{-| A writer to the end of the file, which it creates if there is none.
-}
appender : String -> Fs -> Task Error Stream.Writer
appender =
    Elm.Kernel.NomiFs.appender


{-| A watch on a file or a directory and everything under it: the fiber's
that made it, it stops once that fiber ends.
-}
type Watch
    = Watch


{-| Watches the file or directory at the path, which must exist.

    nextChange : Fs -> Task Nomi.Fs.Error (Maybe { paths : List String })
    nextChange fs =
        Nomi.Fs.watch "src" fs |> Task.andThen Nomi.Fs.change

-}
watch : String -> Fs -> Task Error Watch
watch =
    Elm.Kernel.NomiFs.watch


{-| The paths that changed since the last `change`, from the watched path,
sorted, waiting until one does; `Nothing` once the watch stopped. Changes
nobody has taken yet gather, each path once, so a burst of writes is one
change; past a thousand paths, a change says only `"."`, meaning
something under the watch changed.

The paths are hints, as the system gives them: a change can name a path
that is gone by the time it is read, or one that changed twice, and a
file written by renaming another over it can show as either name. Read
the files to know what they hold.

-}
change : Watch -> Task Error (Maybe { paths : List String })
change =
    Elm.Kernel.NomiFs.change


{-| The directory at the path, as a capability of its own: every path is
now under it, and neither `..` nor a symbolic link leads out of it.

    data : Nomi.Env -> Fs
    data env =
        Nomi.Fs.subdir "data" env.fs

The directory needn't exist yet: its operations fail with `NotFound` until
`makeDirectory` makes it, or a directory in it. A path whose `..`s climb out of the
capability gives one whose every operation fails with `OutsideRoot` and
that path.

On disk, each operation resolves its path's symbolic links when it starts
and refuses one that leads out of the root, or out of any capability it
was narrowed from; files are opened without following a last symbolic
link. That holds against the directory tree as it is when the operation
starts: another process changing it meanwhile, by swapping a directory
for a link, can race it, so a capability is no boundary against someone
else who writes inside it.

-}
subdir : String -> Fs -> Fs
subdir =
    Elm.Kernel.NomiFs.subdir


{-| The same files, refusing every write: `write`, `append`, `remove`,
`makeDirectory`, `rename`, `copy`, `symlink`, `writer` and `appender`
fail with `PermissionDenied` and the path, as they would on a file the
system won't let the program change.
-}
readOnly : Fs -> Fs
readOnly =
    Elm.Kernel.NomiFs.readOnly


{-| A new, empty directory, as a capability of its own: on disk under the
system's temporary directory, and in memory a file system of its own. It
is removed with everything in it once the fiber that made it ends. A
`readOnly` capability makes none (`PermissionDenied`).
-}
temporaryDirectory : Fs -> Task Error Fs
temporaryDirectory =
    Elm.Kernel.NomiFs.temporaryDirectory
