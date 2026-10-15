module Nomi.Process exposing (exit)

{-| The process a `Nomi.program` runs as.

@docs exit

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| Stops the program as SIGINT and SIGTERM do, running every finalizer,
then exits with the code. The task never ends; a program already stopping
keeps the code it was stopping with. Inside `Nomi.Task.uninterruptible`
nothing stops the region, so the process exits when the shutdown deadline
passes.

    Nomi.Process.exit 2

-}
exit : Int -> Task x a
exit =
    Elm.Kernel.Nomi.exitWith
