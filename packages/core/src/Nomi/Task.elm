module Nomi.Task exposing
    ( both, all, race, timeout
    , acquireRelease, ensure, uninterruptible
    , fromResult, fromMaybe
    )

{-| Tasks beyond elm/core's: running tasks concurrently, and resources that
are released however a task ends. elm/core's `Task.map2` and
`Task.sequence` stay sequential.


# Concurrency

@docs both, all, race, timeout


# Resources and interruption

@docs acquireRelease, ensure, uninterruptible


# Conversions

@docs fromResult, fromMaybe

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| Runs both tasks concurrently and succeeds with both values. The first
failure is the result, once the other task is interrupted.
-}
both : Task x a -> Task x b -> Task x ( a, b )
both =
    Elm.Kernel.Nomi.both


{-| Runs the tasks concurrently and succeeds with their values, in the
order of the list. The first failure is the result, once the others are
interrupted.
-}
all : List (Task x a) -> Task x (List a)
all =
    Elm.Kernel.Nomi.all


{-| Runs both tasks concurrently and ends as the first to end does,
succeeding or failing; the other is interrupted.
-}
race : Task x a -> Task x a -> Task x a
race =
    Elm.Kernel.Nomi.race


{-| Gives the task the milliseconds: `Nothing` once they pass, after the
task is interrupted.

    Nomi.Task.timeout 2000 slowQuote

-}
timeout : Int -> Task x a -> Task x (Maybe a)
timeout =
    Elm.Kernel.Nomi.timeout


{-| Acquires a resource, uses it, and releases it however the use ends:
succeeding, failing or interrupted. Acquiring can't be interrupted, so a
resource acquired is always released.

    Nomi.Task.acquireRelease
        (Fiber.fork (flushEvery 5000 data store))
        (\flusher -> Fiber.interrupt flusher |> Task.andThen (\_ -> flush data store))
        (\_ -> serve store)

-}
acquireRelease : Task x r -> (r -> Task Never ()) -> (r -> Task x a) -> Task x a
acquireRelease =
    Elm.Kernel.Nomi.acquireRelease


{-| Runs the finalizer after the task, however the task ends: succeeding,
failing or interrupted.
-}
ensure : Task Never () -> Task x a -> Task x a
ensure =
    Elm.Kernel.Nomi.ensure


{-| Runs the task to its end even when its fiber is interrupted meanwhile;
the interruption takes effect once the task ends. Use it for steps that
must not stop halfway, as clearing a flag and then writing what it marked.
-}
uninterruptible : Task x a -> Task x a
uninterruptible =
    Elm.Kernel.Nomi.uninterruptible


{-| Succeeds with an `Ok`'s value and fails with an `Err`'s.
-}
fromResult : Result x a -> Task x a
fromResult result =
    case result of
        Ok value ->
            Task.succeed value

        Err error ->
            Task.fail error


{-| Succeeds with a `Just`'s value, and fails with the error given for
`Nothing`.
-}
fromMaybe : x -> Maybe a -> Task x a
fromMaybe error maybe =
    case maybe of
        Just value ->
            Task.succeed value

        Nothing ->
            Task.fail error
