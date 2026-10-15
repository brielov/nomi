module Nomi.Stdio exposing (Reader, Writer, line, print, printLine)

{-| The standard streams, which `Nomi.Env` hands a program as `stdin`,
`stdout` and `stderr`.

@docs Reader, Writer, line, print, printLine

-}

import Elm.Kernel.NomiStdio
import Task exposing (Task)


{-| A stream of text to read, line by line.
-}
type Reader
    = Reader


{-| A stream of text to write.
-}
type Writer
    = Writer


{-| The next line, without its `\n` or `\r\n`, or `Nothing` at the end of
the input. The text after the last newline is a line too. Input is read
only while a task waits for a line, and an interrupted `line` loses none.

    lines : Reader -> Task x (List String)
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
line : Reader -> Task x (Maybe String)
line =
    Elm.Kernel.NomiStdio.line


{-| Writes the text, adding no newline, and waits until the stream has
taken it. Output nobody reads any more, as from a pipe into `head`, is
dropped.
-}
print : String -> Writer -> Task x ()
print =
    Elm.Kernel.NomiStdio.print


{-| Writes the text and a newline.
-}
printLine : String -> Writer -> Task x ()
printLine text =
    print (text ++ "\n")
