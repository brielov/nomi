module Nomi.Fake exposing (fs, reader, writer, captured)

{-| nomi/core's file system in memory, scripted input and captured output,
reached from another package's kernel as nomi/test reaches them.

@docs fs, reader, writer, captured

-}

import Elm.Kernel.NomiFake
import Nomi.Fs exposing (Fs)
import Nomi.Stdio exposing (Reader, Writer)
import Task exposing (Task)


{-| A new file system in memory, holding the files.
-}
fs : List ( String, String ) -> Task x Fs
fs =
    Elm.Kernel.NomiFake.fs


{-| A reader of the text, which ends after it.
-}
reader : String -> Task x Reader
reader =
    Elm.Kernel.NomiFake.reader


{-| A writer that keeps what is printed.
-}
writer : Task x Writer
writer =
    Elm.Kernel.NomiFake.writer


{-| What was printed to the writer so far.
-}
captured : Writer -> Task x String
captured =
    Elm.Kernel.NomiFake.captured
