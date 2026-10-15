module Nomi.Queue exposing (Queue, bounded, unbounded, offer, take, poll)

{-| A first-in, first-out queue that fibers share.

A fiber waiting in `take` or `offer` can be interrupted, and leaves the
queue as it was: an interrupted `take` takes nothing, and an interrupted
`offer` adds nothing. Fibers waiting on the same queue are served in the
order they began to wait.

@docs Queue, bounded, unbounded, offer, take, poll

-}

import Elm.Kernel.Nomi
import Task exposing (Task)


{-| A queue of `a`s.
-}
type Queue a
    = Queue


{-| A new, empty queue holding at most that many items, on which `offer`
waits while it is full. With zero, every `offer` waits for a `take`.
-}
bounded : Int -> Task x (Queue a)
bounded =
    Elm.Kernel.Nomi.bounded


{-| A new, empty queue with no limit, on which `offer` never waits.
-}
unbounded : Task x (Queue a)
unbounded =
    Elm.Kernel.Nomi.unbounded


{-| Adds the item at the back of the queue, waiting while it is full.
-}
offer : a -> Queue a -> Task x ()
offer =
    Elm.Kernel.Nomi.offer


{-| Takes the item at the front of the queue, waiting while it is empty.
-}
take : Queue a -> Task x a
take =
    Elm.Kernel.Nomi.take


{-| Takes the item at the front of the queue, or `Nothing` when it is
empty, without waiting.
-}
poll : Queue a -> Task x (Maybe a)
poll =
    Elm.Kernel.Nomi.poll
