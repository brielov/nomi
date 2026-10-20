module Nomi.Stdio exposing
    ( line, print, printLine
    , isTerminal, isInteractive, size
    )

{-| Text on the standard streams, which `Nomi.Env` hands a program as
`stdin`, a `Nomi.Stream.Reader`, and `stdout` and `stderr`, each a
`Nomi.Stream.Writer`. These read and write any stream, in UTF-8.

@docs line, print, printLine


# Terminals

@docs isTerminal, isInteractive, size

-}

import Elm.Kernel.NomiStdio
import Elm.Kernel.NomiStream
import Nomi.Stream as Stream exposing (Reader, Writer)
import Task exposing (Task)


{-| The next line, without its `\n` or `\r\n`, or `Nothing` at the end of
the input. The text after the last newline is a line too. A line that
isn't UTF-8 fails with `Invalid`. Input is read only while a task waits
for it, and the text past a line stays in the reader, for the next line
or the next `Nomi.Stream.read`; an interrupted `line` loses none.

    lines : Reader -> Task Stream.Error (List String)
    lines reader =
        Nomi.Stdio.line reader
            |> Task.andThen
                (\next ->
                    case next of
                        Just text ->
                            lines reader |> Task.map ((::) text)

                        Nothing ->
                            Task.succeed []
                )

-}
line : Reader -> Task Stream.Error (Maybe String)
line =
    Elm.Kernel.NomiStream.readLine


{-| Writes the text in UTF-8, adding no newline, and waits until the
stream has taken it. Output nobody reads any more, as from a pipe into
`head`, is dropped, so writing to `stdout` and `stderr` never fails.
-}
print : String -> Writer -> Task Stream.Error ()
print =
    Elm.Kernel.NomiStdio.print


{-| Writes the text and a newline.
-}
printLine : String -> Writer -> Task Stream.Error ()
printLine text =
    print (text ++ "\n")


{-| Whether the writer writes to a terminal, where colours and a line
rewritten in place make sense, rather than to a file or a pipe.
-}
isTerminal : Writer -> Task x Bool
isTerminal =
    Elm.Kernel.NomiStream.isTerminal


{-| Whether the reader reads a terminal, where a person types, rather than
a file or a pipe.
-}
isInteractive : Reader -> Task x Bool
isInteractive =
    Elm.Kernel.NomiStream.isInteractive


{-| The size of the terminal the writer writes to, in characters, as it is
now: `Nothing` when it isn't a terminal.
-}
size : Writer -> Task x (Maybe { columns : Int, rows : Int })
size =
    Elm.Kernel.NomiStream.size
