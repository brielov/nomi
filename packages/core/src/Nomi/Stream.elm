module Nomi.Stream exposing
    ( Reader, read, readAll, fromBytes, reader, limit
    , Writer, write, close, writer, pipe
    , LineReader, lines, line
    , Error(..), errorToString
    )

{-| Streams of bytes: what a program reads chunk by chunk, as its input, a
file or a body, and what it writes to.

A reader is pulled: its source gives the next chunk only once a `read`
asks for it, so a program reads at the pace it can handle. A write waits
until its writer has taken the bytes, so a slow writer holds back
whatever produces them. Whatever makes a reader or a writer is a task,
since each holds state: a reader the chunks its source gave that no read
took yet, a line reader the text past its last line.

    copy : Reader -> Writer -> Task Error ()
    copy from to =
        Nomi.Stream.read from
            |> Task.andThen
                (\chunk ->
                    case chunk of
                        Just bytes ->
                            Nomi.Stream.write bytes to |> Task.andThen (\_ -> copy from to)

                        Nothing ->
                            Task.succeed ()
                )


# Reading

@docs Reader, read, readAll, fromBytes, reader, limit


# Writing

@docs Writer, write, close, writer, pipe


# Lines

@docs LineReader, lines, line


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiStream
import Nomi.Bytes
import Nomi.Ref as Ref
import Task exposing (Task)


{-| A stream of bytes to read.
-}
type Reader
    = Reader


{-| A stream of bytes to write.
-}
type Writer
    = Writer


{-| A reader read line by line.
-}
type LineReader
    = LineReader


{-| How a stream failed once it was open: the other end gone, a limit
passed, data a reader refuses, or the system's failure. A stream that
failed fails every read after with the same error.
-}
type Error
    = Closed
    | TooLarge
    | Invalid String
    | ConnectionReset
    | PermissionDenied
    | TimedOut
    | Other String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        Closed ->
            "the stream is closed"

        TooLarge ->
            "the stream holds more than its limit"

        Invalid why ->
            "the stream's data is invalid: " ++ why

        ConnectionReset ->
            "the connection was reset"

        PermissionDenied ->
            "permission denied"

        TimedOut ->
            "the stream timed out"

        Other message ->
            message



-- READING


{-| The next chunk of bytes, or `Nothing` at the end of the stream. Reads
are answered in the order they came, each taking the next chunk, and an
interrupted read loses nothing: the chunk its source was giving is the
next read's.
-}
read : Reader -> Task Error (Maybe Bytes)
read =
    Elm.Kernel.NomiStream.read


{-| Everything left in the stream, failing with `TooLarge` once it holds
more than the limit, in bytes.
-}
readAll : { limit : Int } -> Reader -> Task Error Bytes
readAll most from =
    let
        go size chunks =
            read from
                |> Task.andThen
                    (\next ->
                        case next of
                            Nothing ->
                                Task.succeed (Nomi.Bytes.concat (List.reverse chunks))

                            Just chunk ->
                                if size + Nomi.Bytes.length chunk > most.limit then
                                    Task.fail TooLarge

                                else
                                    go (size + Nomi.Bytes.length chunk) (chunk :: chunks)
                    )
    in
    go 0 []


{-| A reader of the bytes, which then ends.
-}
fromBytes : Bytes -> Task x Reader
fromBytes =
    Elm.Kernel.NomiStream.fromBytes


{-| A reader of the chunks the task gives, which ends once it gives
`Nothing`. The task runs only once a read waits for a chunk, one run at a
time, so it streams as any reader does, and a read interrupted while it
runs loses nothing. A failure ends the reader, failing every read after.

The task runs on a fiber of the fiber that made the reader, so the reader
lasts as long as that fiber does: once it ends, reading fails with
`Closed`.

    countdown : Int -> Task x Reader
    countdown from =
        Nomi.Ref.make from
            |> Task.andThen
                (\left ->
                    Nomi.Stream.reader
                        (Nomi.Ref.modify (\n -> ( n, n - 1 )) left
                            |> Task.map
                                (\n ->
                                    if n > 0 then
                                        Just (Nomi.Bytes.fromString (String.fromInt n ++ "\n"))

                                    else
                                        Nothing
                                )
                        )
                )

-}
reader : Task Error (Maybe Bytes) -> Task x Reader
reader =
    Elm.Kernel.NomiStream.fromTask


{-| A reader of what the reader gives, failing with `TooLarge` once it
gave more than the limit, in bytes.
-}
limit : Int -> Reader -> Task x Reader
limit most from =
    Ref.make 0
        |> Task.andThen
            (\count ->
                reader
                    (read from
                        |> Task.andThen
                            (\next ->
                                case next of
                                    Nothing ->
                                        Task.succeed Nothing

                                    Just chunk ->
                                        Ref.modify (\n -> ( n + Nomi.Bytes.length chunk, n + Nomi.Bytes.length chunk )) count
                                            |> Task.andThen
                                                (\size ->
                                                    if size > most then
                                                        Task.fail TooLarge

                                                    else
                                                        Task.succeed (Just chunk)
                                                )
                            )
                    )
            )



-- WRITING


{-| Writes the bytes, succeeding once the writer has taken them. Writes
reach the writer in the order they came. Writing to a closed writer fails
with `Closed`.
-}
write : Bytes -> Writer -> Task Error ()
write =
    Elm.Kernel.NomiStream.write


{-| Closes the writer once the writes before it are done. Closing a closed
writer does nothing.
-}
close : Writer -> Task Error ()
close =
    Elm.Kernel.NomiStream.close


{-| A writer running these tasks: `write` for each write, one at a time
and in order, and `close` once.
-}
writer : { write : Bytes -> Task Error (), close : Task Error () } -> Task x Writer
writer =
    Elm.Kernel.NomiStream.fromTasks


{-| Writes everything the reader gives to the writer, each chunk once the
writer took the last, and leaves the writer open.
-}
pipe : Reader -> Writer -> Task Error ()
pipe from to =
    read from
        |> Task.andThen
            (\next ->
                case next of
                    Just chunk ->
                        write chunk to |> Task.andThen (\_ -> pipe from to)

                    Nothing ->
                        Task.succeed ()
            )



-- LINES


{-| The reader, read by lines. The text past a line stays in the reader,
for the next line or the next `read`.
-}
lines : Reader -> Task x LineReader
lines =
    Elm.Kernel.NomiStream.lines


{-| The next line in UTF-8, without its `\n` or `\r\n`, or `Nothing` at
the end of the stream; the text after the last newline is a line too. A
line that isn't UTF-8 fails with `Invalid`. A line is held whole until
its newline comes, so a stream whose lines may be too long to hold is read
through `limit`.
-}
line : LineReader -> Task Error (Maybe String)
line =
    Elm.Kernel.NomiStream.lineOf
