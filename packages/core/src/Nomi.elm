module Nomi exposing (Program, program, Env, Net, log)

{-| Programs that run on Bun.

@docs Program, program, Env, Net, log

-}

import Dict exposing (Dict)
import Elm.Kernel.Nomi
import Elm.Kernel.NomiStdio
import Nomi.Fs exposing (Fs)
import Nomi.Stream as Stream
import Task exposing (Task)


{-| A program run as a process, with `nomi build` or `nomi run`.
-}
type alias Program =
    Platform.Program () () Never


{-| A program running a task. Its failure is printed to stderr and the
process exits 1; success exits 0. SIGINT and SIGTERM interrupt the task,
running its finalizers, and the process exits 130 or 143.

    main : Nomi.Program
    main =
        Nomi.program (\env -> Task.succeed ())

-}
program : (Env -> Task String ()) -> Program
program run =
    Elm.Kernel.Nomi.program
        (\args vars fs stdin stdout stderr ->
            run
                { args = args
                , vars = Dict.fromList vars
                , fs = fs
                , net = Net
                , stdin = stdin
                , stdout = stdout
                , stderr = stderr
                }
        )


{-| What a program is given: the arguments after its file's name, the
environment's variables, the directory it started in, the network to
serve on, and the standard streams.
-}
type alias Env =
    { args : List String
    , vars : Dict String String
    , fs : Fs
    , net : Net
    , stdin : Stream.Reader
    , stdout : Stream.Writer
    , stderr : Stream.Writer
    }


{-| The capability to listen on the network, which `Nomi.Server.listen`
takes.
-}
type Net
    = Net


{-| Writes the text and a newline to stderr, as `Nomi.Stdio.printLine`
does to `env.stderr`, for a line to whoever runs the program from code
that has no `Env` to hand. Under `nomi test` it goes to the test's
`env.stderr`, which `Nomi.Test.stderr` reads.

    Nomi.log ("could not save: " ++ Nomi.Fs.errorToString error)

-}
log : String -> Task x ()
log =
    Elm.Kernel.NomiStdio.log
