module ProcessTest exposing (suite)

import Expect
import Nomi exposing (Env)
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Process as Process
import Nomi.Test exposing (effect, effectWith)
import Process as Sleep
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "Nomi.Process under nomi test"
        [ effect "a test's processes run no command" <|
            \env ->
                Process.output (Process.command "git" [ "status" ]) env.processes
                    |> Task.map (\_ -> "ran")
                    |> Task.onError (Process.errorToString >> Task.succeed)
                    |> Task.map (Expect.equal "git: not found")
        , effectWith withGit "withCommands answers commands, and ran lists them" ranTwice
        , effectWith withGit "another run of the template starts with none run" ranTwice
        , effect "a fiber waiting for a signal leaves the test's clock free" <|
            \_ ->
                Process.signals [ Process.Hangup ]
                    |> Task.andThen (\listening -> Fiber.fork (Process.signal listening))
                    |> Task.andThen (\_ -> Sleep.sleep 60000)
                    |> Task.andThen (\_ -> Time.now)
                    |> Task.map (Expect.equal (Time.millisToPosix 60000))
        ]


withGit : Env
withGit =
    Nomi.Test.env
        |> Nomi.Test.withCommands
            (\cmd ->
                case ( Process.program cmd, Process.arguments cmd ) of
                    ( "git", [ "rev-parse", "HEAD" ] ) ->
                        Just { status = 0, stdout = Nomi.Bytes.fromString "abc123\n", stderr = Nomi.Bytes.fromString "" }

                    _ ->
                        Nothing
            )


ranTwice : Env -> Task String Expect.Expectation
ranTwice env =
    let
        head =
            Process.output (Process.command "git" [ "rev-parse", "HEAD" ]) env.processes
                |> Task.mapError Process.errorToString
                |> Task.map (.stdout >> Nomi.Bytes.toString)
    in
    Task.map2 Tuple.pair head head
        |> Task.andThen (\both -> Process.ran env.processes |> Task.map (List.map (.command >> Process.arguments) >> Tuple.pair both))
        |> Task.map (Expect.equal ( ( Just "abc123\n", Just "abc123\n" ), [ [ "rev-parse", "HEAD" ], [ "rev-parse", "HEAD" ] ] ))
