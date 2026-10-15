module Nomi exposing (Program, program, Env, Net)

{-| Programs that run on Node.

@docs Program, program, Env, Net

-}

import Dict exposing (Dict)
import Elm.Kernel.Nomi
import Nomi.Fs exposing (Fs)
import Nomi.Stdio as Stdio
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
    , stdin : Stdio.Reader
    , stdout : Stdio.Writer
    , stderr : Stdio.Writer
    }


{-| The capability to listen on the network, which `Nomi.Server.listen`
takes.
-}
type Net
    = Net
