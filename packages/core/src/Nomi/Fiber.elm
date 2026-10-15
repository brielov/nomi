module Nomi.Fiber exposing
    ( Fiber, fork, forkDaemon
    , join, await, interrupt, Exit(..)
    )

{-| Tasks running concurrently, each on a fiber of its own.

A fiber forked with `fork` is its parent's child: it is interrupted, at the
latest, when the parent's task ends. One forked with `forkDaemon` outlives
its parent, as `Process.spawn`'s processes do.

@docs Fiber, fork, forkDaemon


# Ending

Interrupting a fiber runs its finalizers (`Nomi.Task.ensure`,
`Nomi.Task.acquireRelease`) and waits for it. A fiber can be interrupted
between any two steps of its task, so a pair of steps that must not be
parted belongs in `Nomi.Task.uninterruptible`.

@docs join, await, interrupt, Exit

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| A running task, which ends with an error `x` or a value `a`.
-}
type Fiber x a
    = Fiber


{-| Starts the task on a child fiber, interrupted when its parent's task
ends.
-}
fork : Task x a -> Task y (Fiber x a)
fork =
    Elm.Kernel.Nomi.fork


{-| Starts the task on a fiber that outlives its parent.
-}
forkDaemon : Task x a -> Task y (Fiber x a)
forkDaemon =
    Elm.Kernel.Nomi.forkDaemon


{-| Waits for the fiber and ends as it did. A fiber that was interrupted,
or died with a defect, makes the joiner end the same way.
-}
join : Fiber x a -> Task x a
join =
    Elm.Kernel.Nomi.join


{-| Waits for the fiber and succeeds with how it ended.
-}
await : Fiber x a -> Task y (Exit x a)
await =
    Elm.Kernel.Nomi.await


{-| Interrupts the fiber, waits for its finalizers, and succeeds with how
it ended: `Interrupted`, unless it ended first.
-}
interrupt : Fiber x a -> Task y (Exit x a)
interrupt =
    Elm.Kernel.Nomi.interrupt


{-| How a fiber ended: with a value, with an error, interrupted, or with a
defect, a crash such as `Debug.todo` reached or `modBy 0`, described.
-}
type Exit x a
    = Success a
    | Failure x
    | Interrupted
    | Defect String
