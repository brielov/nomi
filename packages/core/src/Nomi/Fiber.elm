module Nomi.Fiber exposing
    ( Fiber, fork, forkDaemon
    , join, await, interrupt, Exit(..)
    )

{-| Tasks running concurrently, each on a fiber of its own.

A fiber forked with `fork` belongs to the innermost scope it is forked in,
and is interrupted, at the latest, when that scope ends. The task of the
fiber that forks it is a scope, and so is each task `Nomi.Task` runs
apart: each of `both`'s, `all`'s and `race`'s tasks, the task `timeout`
limits, the task `ensure` finalizes, and `acquireRelease`'s acquire and use
together. One forked with `forkDaemon` outlives them all, as
`Process.spawn`'s processes do.

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


{-| Starts the task on a child fiber, interrupted when the innermost scope
it is forked in ends. A fiber forked inside `timeout`'s task, say, is
interrupted once that task ends, so joining it after the timeout joins an
interrupted fiber:

    -- `job` is interrupted when the timeout's task ends.
    Nomi.Task.timeout 1000 (Fiber.fork job)

Fork it outside, or with `forkDaemon`, to keep it running.
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
