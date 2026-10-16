module Test.Runner.Effect exposing (outcome)

{-| nomi's: a test's outcome, which the runner reports once the task ends.
Without nomi/test, every test is pure, and its outcome is known at once.
-}

import Expect exposing (Expectation)
import Task exposing (Task)
import Test.Reporter.TestResults exposing (Outcome, outcomeFromExpectations)


outcome : List Expectation -> Task Never Outcome
outcome expectations =
    Task.succeed (outcomeFromExpectations expectations)
