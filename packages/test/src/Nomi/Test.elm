module Nomi.Test exposing (effect)

{-| Tests of tasks, which `nomi test` runs on Node beside pure tests.

@docs effect

-}

import Elm.Kernel.NomiTest
import Expect exposing (Expectation)
import Nomi exposing (Env)
import Task exposing (Task)
import Test exposing (Test)


{-| A test of a task, which gets an `Env` of its own: no arguments or
variables, an empty file system in memory, no input, and output kept in
memory. The test has the expectation the task succeeds with; a task that
fails, fails the test with its error, and one that dies, with its defect.

    effect "a file reads back" <|
        \env ->
            Fs.write "a.txt" "hello" env.fs
                |> Task.andThen (\_ -> Fs.read "a.txt" env.fs)
                |> Task.mapError Fs.errorToString
                |> Task.map (Expect.equal "hello")

-}
effect : String -> (Env -> Task String Expectation) -> Test
effect description run =
    Test.test description <|
        \() -> Elm.Kernel.NomiTest.mark run (Expect.fail "Nomi.Test.effect runs only under nomi test.")
