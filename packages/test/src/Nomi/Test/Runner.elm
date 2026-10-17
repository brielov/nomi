module Nomi.Test.Runner exposing (run)

{-| What `nomi test`'s runner calls. It is not for tests or programs:
it is the hook nomi's runner links, and it changes as the runner does.

@docs run

-}

import Elm.Kernel.NomiTest
import Expect exposing (Expectation)
import Task exposing (Task)


{-| A test's expectations, with each one an effectful test stands for
replaced by what its task gives, run in turn.
-}
run : List Expectation -> Task Never (List Expectation)
run expectations =
    Task.sequence (List.map expectation expectations)


expectation : Expectation -> Task Never Expectation
expectation placeholder =
    case effect placeholder of
        Just test ->
            attempt test
                |> Task.map
                    (\result ->
                        case result of
                            Ok outcome ->
                                outcome

                            Err message ->
                                Expect.fail message
                    )

        Nothing ->
            Task.succeed placeholder


{-| The task an expectation stands for, if it stands for one.
-}
effect : Expectation -> Maybe (Task String Expectation)
effect =
    Elm.Kernel.NomiTest.effect


{-| How the task ended, on a virtual clock of its own: its expectation, or
the message of its failure, of its defect, or of its interruption.
-}
attempt : Task String Expectation -> Task Never (Result String Expectation)
attempt =
    Elm.Kernel.NomiTest.attempt
