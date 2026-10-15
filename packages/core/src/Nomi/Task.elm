module Nomi.Task exposing (ensure)

{-| Tasks beyond elm/core's.

@docs ensure

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| Runs the finalizer after the task, however the task ends: succeeding,
failing or interrupted.
-}
ensure : Task Never () -> Task x a -> Task x a
ensure =
    Elm.Kernel.Nomi.ensure
