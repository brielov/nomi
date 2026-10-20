module Nomi.Process exposing
    ( exit, cwd, pid, platform, Platform(..)
    , Processes, Command, command, withCwd, withEnv, withStdin, withGrace
    , program, arguments, input
    , output, Output
    , Child, spawn, stdin, stdout, stderr, wait, kill
    , Signal(..), Signals, signals, signal
    , fake, ran
    , Error(..), errorToString
    )

{-| The process a `Nomi.program` runs as, and the programs it runs.

@docs exit, cwd, pid, platform, Platform


# Commands

A command is a program and its arguments, given as a list and never a
shell's string, so an argument holding a space, a quote or a `*` reaches
the program as it is.

@docs Processes, Command, command, withCwd, withEnv, withStdin, withGrace
@docs program, arguments, input


# Running

@docs output, Output
@docs Child, spawn, stdin, stdout, stderr, wait, kill


# Signals

@docs Signal, Signals, signals, signal


# Fakes

@docs fake, ran


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Dict exposing (Dict)
import Elm.Kernel.Nomi
import Elm.Kernel.NomiProcess
import Nomi.Bytes
import Nomi.Stream as Stream
import Task exposing (Task)


{-| Stops the program as SIGINT and SIGTERM do, running every finalizer,
then exits with the code. The task never ends; a program already stopping
keeps the code it was stopping with. Inside `Nomi.Task.uninterruptible`
nothing stops the region, so the process exits when the shutdown deadline
passes.

A code runs from 0 to 255; one outside them exits 255, so a failure never
wraps around to another code, or to success.

    Nomi.Process.exit 2

-}
exit : Int -> Task x a
exit =
    Elm.Kernel.Nomi.exitWith


{-| The directory the process runs in, as the system names it.
-}
cwd : Task x String
cwd =
    Elm.Kernel.NomiProcess.cwd


{-| The process's id.
-}
pid : Task x Int
pid =
    Elm.Kernel.NomiProcess.pid


{-| The system the process runs on.
-}
platform : Task x Platform
platform =
    Elm.Kernel.NomiProcess.platform


{-| A system: Linux, macOS, Windows, or another, by the name Bun gives it.
-}
type Platform
    = Linux
    | MacOs
    | Windows
    | OtherPlatform String



-- COMMANDS


{-| The capability to run programs, which `Nomi.Env` hands a program as
`processes`, or a fake of it.
-}
type Processes
    = Processes


{-| A program to run, with its arguments and how to run it.
-}
type Command
    = Command
        { program : String
        , arguments : List String
        , cwd : Maybe String
        , env : Maybe (Dict String String)
        , stdin : Maybe Bytes
        , grace : Int
        }


{-| The program, found as the system finds one without a `/` in its name,
on the `PATH`, and its arguments.

    Nomi.Process.command "git" [ "rev-parse", "HEAD" ]

-}
command : String -> List String -> Command
command name args =
    Command { program = name, arguments = args, cwd = Nothing, env = Nothing, stdin = Nothing, grace = 5000 }


{-| Runs the command in the directory, from the one the process runs in,
rather than in that one.
-}
withCwd : String -> Command -> Command
withCwd dir (Command c) =
    Command { c | cwd = Just dir }


{-| Runs the command with these variables, and no other, rather than with
the process's.

    Nomi.Process.command "make" [] |> Nomi.Process.withEnv (Dict.insert "CC" "clang" env.vars)

-}
withEnv : Dict String String -> Command -> Command
withEnv vars (Command c) =
    Command { c | env = Just vars }


{-| Gives the command these bytes as its standard input, which then ends.
Without them, `output` gives it an input that has ended, and `spawn` a
`stdin` writer.
-}
withStdin : Bytes -> Command -> Command
withStdin bytes (Command c) =
    Command { c | stdin = Just bytes }


{-| How long, in milliseconds, a command whose fiber ended has to exit
once it is sent SIGTERM, before it is sent SIGKILL: 5 seconds unless
given.
-}
withGrace : Int -> Command -> Command
withGrace ms (Command c) =
    Command { c | grace = ms }


{-| The command's program, as a fake's function reads it.
-}
program : Command -> String
program (Command c) =
    c.program


{-| The command's arguments.
-}
arguments : Command -> List String
arguments (Command c) =
    c.arguments


{-| The bytes `withStdin` gave the command, or none.
-}
input : Command -> Bytes
input (Command c) =
    Maybe.withDefault (Nomi.Bytes.fromString "") c.stdin



-- RUNNING


{-| What a command that ran to its end gave: its status, 0 for success,
or 128 plus the signal's number for one a signal ended, as a shell says,
and all it wrote.
-}
type alias Output =
    { status : Int
    , stdout : Bytes
    , stderr : Bytes
    }


{-| Runs the command to its end. A status other than 0 is data, not a
failure; the task fails only when the command can't run, as when there is
no such program. Interrupting the task ends the command as its fiber
ending does (`spawn`).

    Nomi.Process.output (Nomi.Process.command "git" [ "status", "--short" ]) env.processes

-}
output : Command -> Processes -> Task Error Output
output ((Command c) as cmd) =
    Elm.Kernel.NomiProcess.output cmd c


{-| A running command.
-}
type Child
    = Child


{-| Starts the command. The child is the fiber's that started it: once
that fiber ends, however it ends, a child still running is sent SIGTERM,
and SIGKILL if it hasn't exited within its grace (`withGrace`); on
Windows, which has no SIGTERM, it ends at once. Its output is read
through `stdout` and `stderr`, and a child whose output nobody reads waits
once the pipe is full.
-}
spawn : Command -> Processes -> Task Error Child
spawn ((Command c) as cmd) =
    Elm.Kernel.NomiProcess.spawn cmd c


{-| What writes to the child's standard input; closing it ends the input.
With `withStdin`, it is closed already.
-}
stdin : Child -> Stream.Writer
stdin =
    Elm.Kernel.NomiProcess.stdin


{-| What reads the child's standard output.
-}
stdout : Child -> Stream.Reader
stdout =
    Elm.Kernel.NomiProcess.stdout


{-| What reads the child's standard error.
-}
stderr : Child -> Stream.Reader
stderr =
    Elm.Kernel.NomiProcess.stderr


{-| Waits for the child to exit and succeeds with its status, as `output`
gives one. Interrupting the wait leaves the child running.
-}
wait : Child -> Task Error Int
wait =
    Elm.Kernel.NomiProcess.wait


{-| Sends the child the signal. A child that exited already is left as it
is. On Windows every signal ends the child at once.
-}
kill : Signal -> Child -> Task Error ()
kill =
    Elm.Kernel.NomiProcess.kill



-- SIGNALS


{-| A signal, by what it is for: SIGHUP, SIGINT, SIGQUIT, SIGKILL, SIGUSR1,
SIGUSR2, SIGTERM and SIGWINCH.
-}
type Signal
    = Hangup
    | Interrupt
    | Quit
    | Kill
    | User1
    | User2
    | Terminate
    | WindowChanged


{-| Signals the process receives, which a fiber reads with `signal`.
-}
type Signals
    = Signals


{-| Listens for these signals, from now until the fiber that listens ends.
SIGINT and SIGTERM still stop the program, as they do when nobody listens;
SIGKILL never arrives, as no process can catch it, and neither does a
signal the system doesn't have, as SIGHUP on Windows. A listener only
listens: a signal whose default would end the process, as SIGHUP's does,
is the listener's to act on.

    nextHangup : Task x (Maybe Nomi.Process.Signal)
    nextHangup =
        Nomi.Process.signals [ Nomi.Process.Hangup ] |> Task.andThen Nomi.Process.signal

-}
signals : List Signal -> Task x Signals
signals =
    Elm.Kernel.NomiProcess.signals


{-| The next signal, oldest first, waiting until one comes; `Nothing` once
the fiber that listened ended. Signals nobody has taken gather, each kind
once.
-}
signal : Signals -> Task x (Maybe Signal)
signal =
    Elm.Kernel.NomiProcess.signal



-- FAKES


{-| Processes that run nothing: each command gets what the function gives
for it, its output as `output`'s, and `Nothing` is a program there is
none of (`NotFound`). A spawned child gives the output as its `stdout` and
`stderr`, keeps what is written to its `stdin`, and exits once that is
closed.

    Nomi.Process.fake
        (\cmd ->
            case ( Nomi.Process.program cmd, Nomi.Process.arguments cmd ) of
                ( "git", [ "rev-parse", "HEAD" ] ) ->
                    Just { status = 0, stdout = Nomi.Bytes.fromString "abc123\n", stderr = Nomi.Bytes.fromString "" }

                _ ->
                    Nothing
        )

-}
fake : (Command -> Maybe Output) -> Task x Processes
fake =
    Elm.Kernel.NomiProcess.fake


{-| The commands a fake ran, in order, each with what it was given on its
standard input: by `withStdin`, or written to a spawned child's `stdin`
so far. Real processes ran none it keeps.
-}
ran : Processes -> Task x (List { command : Command, stdin : Bytes })
ran =
    Elm.Kernel.NomiProcess.ran



-- ERRORS


{-| Why a command didn't run: no such program, or working directory
(`withCwd`), one the system won't run, or another reason.
-}
type Error
    = NotFound String
    | PermissionDenied String
    | Other String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        NotFound name ->
            name ++ ": not found"

        PermissionDenied name ->
            name ++ ": permission denied"

        Other message ->
            message
