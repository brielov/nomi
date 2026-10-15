module Nomi exposing (Program, program, Env, Net)

{-| Programs that run on Node.

@docs Program, program, Env, Net

-}

import Dict exposing (Dict)
import Elm.Kernel.Nomi
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
        (\args vars -> run { args = args, vars = Dict.fromList vars, net = Net })


{-| What a program is given: the arguments after its file's name, the
environment's variables, and the network to serve on.
-}
type alias Env =
    { args : List String
    , vars : Dict String String
    , net : Net
    }


{-| The capability to listen on the network, which `Nomi.Server.listen`
takes.
-}
type Net
    = Net
