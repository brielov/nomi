module Test.Runner.Effect exposing (outcome)

{-| nomi's: a test's outcome, which the runner reports once the task ends.
With nomi/test, an effectful test's expectation stands for a task, run
here on Node.
-}

import Expect exposing (Expectation)
import Nomi.Test.Runner
import Task exposing (Task)
import Test.Reporter.TestResults exposing (Outcome, outcomeFromExpectations)


outcome : List Expectation -> Task Never Outcome
outcome expectations =
    Nomi.Test.Runner.run expectations
        |> Task.map outcomeFromExpectations
