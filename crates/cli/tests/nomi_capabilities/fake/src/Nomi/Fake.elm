module Nomi.Fake exposing (fs, reader, writer, captured)

{-| nomi/core's file system in memory, scripted input and captured output,
reached from another package's kernel as nomi/test reaches them.

@docs fs, reader, writer, captured

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiFake
import Nomi.Fs exposing (Fs)
import Nomi.Stream exposing (Reader, Writer)
import Task exposing (Task)


{-| A new file system in memory, holding the files.
-}
fs : List ( String, String ) -> Task x Fs
fs =
    Elm.Kernel.NomiFake.fs


{-| A reader of the bytes, which ends after them.
-}
reader : Bytes -> Task x Reader
reader =
    Elm.Kernel.NomiFake.reader


{-| A writer that keeps what is written.
-}
writer : Task x Writer
writer =
    Elm.Kernel.NomiFake.writer


{-| What was written to the writer so far, as text.
-}
captured : Writer -> Task x String
captured =
    Elm.Kernel.NomiFake.captured
