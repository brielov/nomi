module Nomi.Ref exposing (Ref, make, get, set, update, modify)

{-| A mutable cell that fibers share.

Fibers take turns on one thread, so a `Ref` needs no lock; but another
fiber can run between a `get` and the `set` after it, so a change that
reads the value belongs in `update` or `modify`, which are one step.

@docs Ref, make, get, set, update, modify

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| A cell holding an `a`.
-}
type Ref a
    = Ref


{-| A new cell holding the value.
-}
make : a -> Task x (Ref a)
make =
    Elm.Kernel.Nomi.makeRef


{-| The value the cell holds.
-}
get : Ref a -> Task x a
get =
    Elm.Kernel.Nomi.getRef


{-| Replaces the value the cell holds.
-}
set : a -> Ref a -> Task x ()
set =
    Elm.Kernel.Nomi.setRef


{-| Applies the function to the value the cell holds, in one step.
-}
update : (a -> a) -> Ref a -> Task x ()
update =
    Elm.Kernel.Nomi.updateRef


{-| Applies the function to the value the cell holds, in one step: the
cell keeps the pair's second part, and the task succeeds with its first.

    next : Ref Int -> Task x Int
    next counter =
        Nomi.Ref.modify (\n -> ( n, n + 1 )) counter

-}
modify : (a -> ( b, a )) -> Ref a -> Task x b
modify =
    Elm.Kernel.Nomi.modifyRef
