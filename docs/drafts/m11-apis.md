# M11 API drafts

Drafts for the review that opens M11: package names and APIs, judged by the server written against them at the end. Nothing here is implemented yet. Each **Decision** names what is open and the draft's choice.

## Packages

| Package | Modules | Holds |
|---|---|---|
| `nomi/core` | `Nomi`, `Nomi.Task`, `Nomi.Fiber`, `Nomi.Ref`, `Nomi.Queue`, `Nomi.Fs`, `Nomi.Process`, `Nomi.Stdio`, `Nomi.Ffi` | programs, concurrency, capabilities, FFI |
| `nomi/server` | `Nomi.Server` | HTTP serving |
| `nomi/test` | `Nomi.Test` | effectful tests, fake capabilities, the virtual clock |

**Decision: three packages, not one per plan item.** `Env` names every capability's type, so capabilities can't leave the package that defines `Env` without a cycle; concurrency is what every capability returns. The server is apart because most programs (scripts, workers, CLIs) never serve, and `nomi/test` because only test dependencies need it. All three ship with the toolchain and version together.

**Decision: the `Nomi.` prefix on every module.** It keeps `Task`, `Process` and `Http` free for elm/core and elm/http, which a server imports beside these (`elm/core` already has a `Process` module; `Nomi.Process` doesn't clash with it).

## `Nomi` (nomi/core)

```elm
type alias Program =
    Platform.Program () () Never

program : (Env -> Task String ()) -> Program

type alias Env =
    { args : List String
    , vars : Dict String String
    , fs : Fs
    , net : Net
    , stdin : Stdio.Reader
    , stdout : Stdio.Writer
    , stderr : Stdio.Writer
    }
```

- The task's failure is printed to stderr and the process exits 1; success exits 0; SIGINT and SIGTERM interrupt it (130, 143), finalizers first.
- **Decision: `Env` is a record alias**, not an opaque type: fields read as `env.fs`, and `{ env | fs = Fs.subdir "data" env.fs }` narrows what a function is handed. A new capability is a new field, which breaks only code that builds an `Env` by hand; tests get one from `Nomi.Test.env`.
- **Decision: `Task String ()` for the program.** A custom error type would make every program write an `errorToString` it already has; `String` is what reaches the terminal anyway.

## `Nomi.Task`

elm/core's `Task` is kept as upstream defines it (`map2` and `sequence` stay sequential); parallel and resource combinators live here.

```elm
both : Task x a -> Task x b -> Task x ( a, b )        -- concurrent; the first failure interrupts the other
all : List (Task x a) -> Task x (List a)              -- concurrent, results in order
race : Task x a -> Task x a -> Task x a               -- first to finish wins; the loser is interrupted
timeout : Int -> Task x a -> Task x (Maybe a)         -- Nothing after the milliseconds pass; the task is interrupted

acquireRelease : Task x r -> (r -> Task Never ()) -> (r -> Task x a) -> Task x a
ensure : Task Never () -> Task x a -> Task x a        -- runs the finalizer however the task ends
uninterruptible : Task x a -> Task x a

fromResult : Result x a -> Task x a
fromMaybe : x -> Maybe a -> Task x a
```

- **Decision: `acquireRelease` takes the use function** (bracket style) rather than registering in an ambient scope. Elm has no scope to name, so the resource's lifetime is the use function's; the runtime's scopes do the work underneath.
- **Decision: `race` takes two tasks.** A list version needs a story for the empty list; `race a (race b c)` covers more.

## `Nomi.Fiber`

```elm
type Fiber x a

fork : Task x a -> Task y (Fiber x a)        -- a child: interrupted when its parent's scope ends
forkDaemon : Task x a -> Task y (Fiber x a)  -- outlives its parent, as Process.spawn does
join : Fiber x a -> Task x a                 -- waits; a defect or interruption of the child is the joiner's
await : Fiber x a -> Task y (Exit x a)
interrupt : Fiber x a -> Task y (Exit x a)

type Exit x a
    = Success a
    | Failure x
    | Interrupted
    | Defect String
```

## `Nomi.Ref` and `Nomi.Queue`

```elm
type Ref a

make : a -> Task x (Ref a)
get : Ref a -> Task x a
set : a -> Ref a -> Task x ()
update : (a -> a) -> Ref a -> Task x ()
modify : (a -> ( b, a )) -> Ref a -> Task x b      -- read and write as one step


type Queue a

bounded : Int -> Task x (Queue a)                  -- offer waits while full
unbounded : Task x (Queue a)
offer : a -> Queue a -> Task x ()
take : Queue a -> Task x a                         -- waits while empty
poll : Queue a -> Task x (Maybe a)
```

Fibers are cooperative on one thread, so a `Ref` needs no locks; `modify` exists because a `get` then `set` can be interleaved by another fiber at the `andThen` between them.

## Capabilities

```elm
-- Nomi.Fs
type Fs

read : String -> Fs -> Task Error String
readBytes : String -> Fs -> Task Error Bytes
write : String -> String -> Fs -> Task Error ()
append : String -> String -> Fs -> Task Error ()
list : String -> Fs -> Task Error (List String)
remove : String -> Fs -> Task Error ()
makeDirectory : String -> Fs -> Task Error ()
exists : String -> Fs -> Task x Bool
subdir : String -> Fs -> Fs                        -- every path is now under it; `..` cannot leave it
readOnly : Fs -> Fs

type Error
    = NotFound String
    | PermissionDenied String
    | AlreadyExists String
    | IsADirectory String
    | OutsideRoot String
    | Other String

errorToString : Error -> String


-- Nomi.Stdio
type Reader
type Writer

line : Reader -> Task x (Maybe String)            -- Nothing at end of input
print : String -> Writer -> Task x ()              -- no newline added
printLine : String -> Writer -> Task x ()


-- Nomi.Process
exit : Int -> Task x a                             -- finalizers run first
```

- **Decision: the capability comes last** (`Fs.read path env.fs`, `env.fs |> Fs.read path`), as elm/core puts the data structure last.
- **Decision: paths are `String`s**, relative to the capability's root, `/`-separated on every platform.
- `Net` is `Nomi.Server`'s; `Nomi` re-exposes it only as `Env`'s field type.

## `Nomi.Ffi`

```elm
task : String -> Json.Encode.Value -> Json.Decode.Decoder a -> Task Error a

type Error
    = Threw String          -- the JS function threw or rejected
    | BadResult String      -- the decoder failed
```

The JS side is an async function exported from the entry's `ffi` module, called as `f(arg, signal)`; interruption aborts `signal`. A name written as a literal is checked against the module's exports when linking.

## `Nomi.Server` (nomi/server)

```elm
type Net

listen : Config -> (Request -> Task Never Response) -> Net -> Task Error ()

type alias Config =
    { port_ : Int
    , host : String              -- "127.0.0.1"; "0.0.0.0" to listen everywhere
    , drainMilliseconds : Int    -- how long in-flight requests may finish after SIGTERM
    }

defaults : Config                -- port 8080, 127.0.0.1, 10 seconds


-- Requests
type Request

method : Request -> Method
path : Request -> List String           -- "/todos/3" is [ "todos", "3" ]
query : String -> Request -> Maybe String
header : String -> Request -> Maybe String   -- case-insensitive
body : Request -> Task BodyError String
jsonBody : Decoder a -> Request -> Task BodyError a

type Method = Get | Head | Post | Put | Patch | Delete | Options | OtherMethod String

type BodyError = TooLarge | BadJson String | Disconnected


-- Responses
type Response

text : Int -> String -> Response
json : Int -> Json.Encode.Value -> Response
empty : Int -> Response
withHeader : String -> String -> Response -> Response


type Error = AddressInUse | PermissionDenied | OtherError String

errorToString : Error -> String
```

- `listen` runs until the program is interrupted, then drains: it stops accepting, lets in-flight requests run until `drainMilliseconds` pass, interrupts the rest and closes.
- Each request runs on its own fiber in the server's scope. A client that disconnects interrupts it; a defect answers 500 and writes a log line to stderr.
- **Decision: the handler is `Request -> Task Never Response`**, fetch-shaped, so every error becomes a response in the handler and the type says so.
- **Decision: `port_`**, since `port` is an Elm keyword.
- **Decision: no router.** `case ( method req, path req ) of` is the router; the example below shows it reads well.

## `Nomi.Test` (nomi/test)

```elm
effect : String -> (Env -> Task String Expectation) -> Test
env : Env                                    -- a memory file system, empty stdin, captured output
withFiles : List ( String, String ) -> Env -> Env
withArgs : List String -> Env -> Env
stdout : Env -> Task x String                -- what was printed so far
advance : Int -> Task x ()                   -- moves the virtual clock; Process.sleep and Time obey it
```

Every effectful test runs under the virtual clock: time only moves when every fiber waits on a timer, or on `advance`.

## The example, written against the drafts

A small todo API: a JSON file for storage, a `Ref` for the live state, a background fiber that writes the file every five seconds when it changed, a timeout on a slow dependency, and a graceful shutdown that flushes once more.

```elm
module Main exposing (main)

import Dict exposing (Dict)
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Nomi exposing (Env)
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Ref as Ref exposing (Ref)
import Nomi.Server as Server exposing (Method(..), Request, Response)
import Nomi.Stdio as Stdio
import Nomi.Task
import Process
import Task exposing (Task)


type alias Todo =
    { title : String, done : Bool }


type alias Store =
    { todos : Dict Int Todo, next : Int, dirty : Bool }


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                data =
                    Fs.subdir "data" env.fs
            in
            load data
                |> Task.andThen Ref.make
                |> Task.andThen
                    (\store ->
                        Nomi.Task.acquireRelease
                            (Fiber.fork (flushEvery 5000 data store))
                            (\flusher -> Fiber.interrupt flusher |> Task.andThen (\_ -> flush data store))
                            (\_ ->
                                Stdio.printLine "listening on 8080" env.stdout
                                    |> Task.andThen (\_ -> Server.listen Server.defaults (handle store) env.net)
                                    |> Task.mapError Server.errorToString
                            )
                    )


load : Fs -> Task String Store
load fs =
    Fs.read "todos.json" fs
        |> Task.map (Decode.decodeString decoder >> Result.withDefault Dict.empty)
        |> Task.onError
            (\error ->
                case error of
                    Fs.NotFound _ ->
                        Task.succeed Dict.empty

                    _ ->
                        Task.fail (Fs.errorToString error)
            )
        |> Task.map (\todos -> { todos = todos, next = nextId todos, dirty = False })


flushEvery : Float -> Fs -> Ref Store -> Task x ()
flushEvery ms fs store =
    Process.sleep ms
        |> Task.andThen (\_ -> flush fs store)
        |> Task.andThen (\_ -> flushEvery ms fs store)


flush : Fs -> Ref Store -> Task x ()
flush fs store =
    -- Uninterruptible: interrupted between clearing `dirty` and writing,
    -- the shutdown's last flush would find nothing to write.
    Nomi.Task.uninterruptible <|
        (Ref.modify (\s -> ( s, { s | dirty = False } )) store
            |> Task.andThen
                (\s ->
                    if s.dirty then
                        Fs.write "todos.json" (Encode.encode 2 (encode s.todos)) fs
                            |> Task.onError (\_ -> Task.succeed ())

                    else
                        Task.succeed ()
                )
        )


handle : Ref Store -> Request -> Task Never Response
handle store req =
    case ( Server.method req, Server.path req ) of
        ( Get, [ "todos" ] ) ->
            Ref.get store
                |> Task.map (\s -> Server.json 200 (encode s.todos))

        ( Post, [ "todos" ] ) ->
            Server.jsonBody (Decode.field "title" Decode.string) req
                |> Task.andThen
                    (\title ->
                        Ref.modify
                            (\s -> ( s.next, { s | todos = Dict.insert s.next (Todo title False) s.todos, next = s.next + 1, dirty = True } ))
                            store
                    )
                |> Task.map (\id -> Server.json 201 (Encode.object [ ( "id", Encode.int id ) ]))
                |> Task.onError (\_ -> Task.succeed (Server.text 400 "expected {\"title\": string}"))

        ( Delete, [ "todos", id ] ) ->
            case String.toInt id of
                Just n ->
                    Ref.update (\s -> { s | todos = Dict.remove n s.todos, dirty = True }) store
                        |> Task.map (\_ -> Server.empty 204)

                Nothing ->
                    Task.succeed (Server.text 404 "no such todo")

        ( Get, [ "quote" ] ) ->
            -- A slow dependency gets two seconds.
            Nomi.Task.timeout 2000 slowQuote
                |> Task.map (Maybe.withDefault "no quote today" >> Server.text 200)

        _ ->
            Task.succeed (Server.text 404 "not found")


slowQuote : Task x String
slowQuote =
    Process.sleep 3000 |> Task.map (\_ -> "patience")


nextId : Dict Int Todo -> Int
nextId todos =
    Dict.keys todos |> List.maximum |> Maybe.map ((+) 1) |> Maybe.withDefault 1


decoder : Decoder (Dict Int Todo)
decoder =
    Decode.keyValuePairs (Decode.map2 Todo (Decode.field "title" Decode.string) (Decode.field "done" Decode.bool))
        |> Decode.map (List.filterMap (\( k, v ) -> String.toInt k |> Maybe.map (\n -> ( n, v ))) >> Dict.fromList)


encode : Dict Int Todo -> Encode.Value
encode todos =
    todos
        |> Dict.toList
        |> List.map (\( id, t ) -> ( String.fromInt id, Encode.object [ ( "title", Encode.string t.title ), ( "done", Encode.bool t.done ) ] ))
        |> Encode.object
```

## What writing it showed

- **Interruption is the hard part, as expected.** The first draft of `flush` cleared `dirty` and then wrote; the shutdown interrupting the flusher between the two would have lost the last changes, since the final flush then saw nothing dirty. `uninterruptible` fixed it, and nothing in the types pointed at it. The docs of `Fiber.interrupt` and `acquireRelease` should show this case.
- **It reads acceptably.** The longest chain is `main`'s, and most of its length is `acquireRelease`'s three arguments. `Ref.modify` carried every read-and-write; `get` then `set` was never needed.
- **`Process.sleep` takes a `Float`**, elm/core's type, so `flushEvery` does too; nomi's own APIs take `Int` milliseconds. Kept: changing elm/core is out, and an `Int` is what a server means.
- **`Task Never Response` forced every error into a response** at the place it happened, which is what a server wants; `Server.jsonBody`'s `BodyError` was mapped to a 400 in one line.
- **Errors become `String` at the edge** (`Server.errorToString`, `Fs.errorToString`), as `Task String ()` intends; nothing needed a custom program error.
- **Missing:** a way to log without threading `env.stderr` everywhere. Draft: `Nomi.log : String -> Task x ()`, writing a line to stderr, the one ambient effect besides elm/*'s. It is not in the signatures above until the review agrees.
