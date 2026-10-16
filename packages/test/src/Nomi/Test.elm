module Nomi.Test exposing
    ( effect, effectWith
    , env, withFiles, withArgs, stdout, stderr
    , advance
    )

{-| Tests of tasks, which `nomi test` runs on Node beside pure tests.

Every effectful test runs on a virtual clock of its own, from 0: its time
moves only when every one of its fibers waits on a timer, or on `advance`,
so `Process.sleep` and `Time.now` take no real time. What the test leaves
running once its task ends is interrupted.

@docs effect, effectWith


# Environments

@docs env, withFiles, withArgs, stdout, stderr


# Time

@docs advance

-}

import Dict
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
effect =
    effectWith env


{-| `effect`, with the test's `Env` made from the one given: each run of
the test gets a copy of its file system, no input and new captured output.

    effectWith (Nomi.Test.env |> withArgs [ "config.json" ] |> withFiles [ ( "config.json", "{}" ) ])
        "the program reads the file it is given"
    <|
        \env ->
            Main.run env
                |> Task.andThen (\_ -> stdout env)
                |> Task.map (Expect.equal "ok\n")

-}
effectWith : Env -> String -> (Env -> Task String Expectation) -> Test
effectWith template description run =
    Test.test description <|
        \() ->
            Elm.Kernel.NomiTest.mark
                (Task.andThen run (fresh template))
                (Expect.fail "Nomi.Test.effect runs only under nomi test.")


{-| A copy of `template` for one run of a test.
-}
fresh : Env -> Task x Env
fresh template =
    Elm.Kernel.NomiTest.fresh
        (\fs stdin out err -> { template | fs = fs, stdin = stdin, stdout = out, stderr = err })
        template.fs


{-| The `Env` an `effect` gets: no arguments or variables, an empty file
system in memory, an input that has ended, and output kept in memory.
-}
env : Env
env =
    Elm.Kernel.NomiTest.env
        (\fs stdin out err net ->
            { args = []
            , vars = Dict.empty
            , fs = fs
            , net = net
            , stdin = stdin
            , stdout = out
            , stderr = err
            }
        )


{-| `env` with these files, each a path and its text, beside what its file
system already holds, and the directories they need.
-}
withFiles : List ( String, String ) -> Env -> Env
withFiles files environment =
    { environment | fs = Elm.Kernel.NomiTest.withFiles files environment.fs }


{-| `env` with these arguments.
-}
withArgs : List String -> Env -> Env
withArgs args environment =
    { environment | args = args }


{-| What was printed to `env.stdout` so far.
-}
stdout : Env -> Task x String
stdout environment =
    Elm.Kernel.NomiTest.captured environment.stdout


{-| What was printed to `env.stderr` so far, with what `Nomi.log` wrote
during the test.
-}
stderr : Env -> Task x String
stderr environment =
    Elm.Kernel.NomiTest.captured environment.stderr


{-| Waits this many milliseconds of the test's clock, firing every timer
due by then in order, and lets what they woke run until each of the
test's fibers waits again.
-}
advance : Int -> Task x ()
advance =
    Elm.Kernel.NomiTest.advance
