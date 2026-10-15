module Nomi.Fs exposing
    ( Fs
    , read, readBytes, write, append, list, remove, makeDirectory, exists
    , subdir, readOnly
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

@docs read, readBytes, write, append, list, remove, makeDirectory, exists


# Narrowing

@docs subdir, readOnly


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiFs
import Task exposing (Task)


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

        OutsideRoot path ->
            path ++ ": outside the root"

        Other message ->
            message


{-| The file's text, decoded as UTF-8.

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
replacing what it held. The directory it is in must exist.
-}
write : String -> String -> Fs -> Task Error ()
write =
    Elm.Kernel.NomiFs.write


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


{-| Removes the file, or the directory with everything in it. A symbolic
link is removed, not what it leads to; the root is never removed
(`PermissionDenied`).
-}
remove : String -> Fs -> Task Error ()
remove =
    Elm.Kernel.NomiFs.remove


{-| Creates the directory, and those it is in that don't exist yet. If it
already exists, the task fails with `AlreadyExists`.
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


{-| The directory at the path, as a capability of its own: every path is
now under it, and neither `..` nor a symbolic link leads out of it.

    data : Nomi.Env -> Fs
    data env =
        Nomi.Fs.subdir "data" env.fs

The directory needn't exist yet. A path whose `..`s climb out of the
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


{-| The same files, refusing every write: `write`, `append`, `remove` and
`makeDirectory` fail with `PermissionDenied` and the path, as they would on
a file the system won't let the program change.
-}
readOnly : Fs -> Fs
readOnly =
    Elm.Kernel.NomiFs.readOnly
