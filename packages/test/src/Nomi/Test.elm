module Nomi.Test exposing
    ( effect, effectWith
    , env, withFiles, withArgs, withVars, withStdin, stdout, stderr
    , advance
    )

{-| Tests of tasks, which `nomi test` runs on Node beside pure tests.

Every effectful test runs on a virtual clock of its own, from 0: its time
moves only when every one of its fibers waits on a timer, or on `advance`,
so `Process.sleep` and `Time.now` take no real time. What the test leaves
running once its task ends is interrupted.

A server can listen inside a test, on the test's runtime, with elm/http as
its client. A request on the wire is the host's, which the clock waits
for: while one is in flight, no timer fires, so a handler that sleeps or
times out is tested with `Nomi.Server.respond` instead.

@docs effect, effectWith


# Environments

@docs env, withFiles, withArgs, withVars, withStdin, stdout, stderr


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
the test gets a copy of its file system, its input from the start, and new
captured output.

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
        template.stdin


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


{-| `env` with these arguments, in place of those it has.
-}
withArgs : List String -> Env -> Env
withArgs args environment =
    { environment | args = args }


{-| `env` with these variables, each a name and its value, beside those it
has; a name it has already takes the value given.
-}
withVars : List ( String, String ) -> Env -> Env
withVars vars environment =
    { environment | vars = Dict.union (Dict.fromList vars) environment.vars }


{-| `env` with this text as its input, in place of the input it has; the
input ends after the text.

    effectWith (Nomi.Test.env |> withStdin "milk\neggs\n") "the list is read to its end" <|
        \env -> Main.shopping env |> Task.map (Expect.equal [ "milk", "eggs" ])

-}
withStdin : String -> Env -> Env
withStdin text environment =
    { environment | stdin = Elm.Kernel.NomiTest.stdin text }


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
