module Nomi.Test.Runner exposing (run)

{-| What `nomi test`'s runner calls; tests don't need it.

@docs run

-}

import Dict
import Elm.Kernel.NomiTest
import Expect exposing (Expectation)
import Nomi exposing (Env)
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
            attempt (Task.andThen test env)
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


{-| The test an expectation stands for, if it stands for one.
-}
effect : Expectation -> Maybe (Env -> Task String Expectation)
effect =
    Elm.Kernel.NomiTest.effect


{-| How the task ended: its expectation, or the message of its failure,
of its defect, or of its interruption.
-}
attempt : Task String Expectation -> Task Never (Result String Expectation)
attempt =
    Elm.Kernel.NomiTest.attempt


{-| A new `Env` for a test.
-}
env : Task x Env
env =
    Elm.Kernel.NomiTest.env
        (\fs stdin stdout stderr net ->
            { args = []
            , vars = Dict.empty
            , fs = fs
            , net = net
            , stdin = stdin
            , stdout = stdout
            , stderr = stderr
            }
        )
