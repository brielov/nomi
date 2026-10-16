module StoreTest exposing (suite)

import Dict
import Expect
import Main exposing (Store, Todo)
import Nomi
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs
import Nomi.Ref as Ref
import Nomi.Test exposing (advance, effect, effectWith, stdout, withFiles)
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "the store"
        [ effect "no file is no todos" <|
            \env ->
                Main.load env.fs
                    |> Task.map (Expect.equal { todos = Dict.empty, next = 1, dirty = False })
        , effectWith (Nomi.Test.env |> withFiles [ ( "todos.json", saved ) ]) "a saved file loads back" <|
            \env ->
                Main.load env.fs
                    |> Task.map (Expect.equal { todos = Dict.fromList [ ( 2, Todo "milk" False ), ( 7, Todo "eggs" True ) ], next = 8, dirty = False })
        , effectWith (Nomi.Test.env |> withFiles [ ( "todos.json", "[]" ) ]) "a file that isn't todos fails to load" <|
            \env ->
                Main.load env.fs
                    |> Task.map (\_ -> Expect.fail "it loaded")
                    |> Task.onError (\error -> Task.succeed (Expect.equal True (String.startsWith "todos.json: " error)))
        , effect "the flusher writes what changed every five seconds" <|
            \env ->
                Ref.make (store False)
                    |> Task.andThen (\ref -> Fiber.fork (Main.flushEvery 5000 env.stderr env.fs ref) |> Task.map (\_ -> ref))
                    |> Task.andThen
                        (\ref ->
                            advance 5000
                                |> Task.andThen (\_ -> Ref.set (store True) ref)
                                |> Task.andThen (\_ -> advance 4999)
                                |> Task.andThen (\_ -> Fs.exists "todos.json" env.fs)
                                |> Task.andThen (\early -> advance 1 |> Task.andThen (\_ -> readBack env) |> Task.map (Tuple.pair early))
                        )
                    |> Task.map (Expect.equal ( False, Ok (store False) ))
        , effect "a write that fails is reported and tried again" <|
            \env ->
                Ref.make (store True)
                    |> Task.andThen
                        (\ref ->
                            Main.flush env.stdout (Fs.readOnly env.fs) ref
                                |> Task.andThen (\_ -> Main.flush env.stdout env.fs ref)
                                |> Task.andThen (\_ -> Task.map2 Tuple.pair (stdout env) (readBack env))
                        )
                    |> Task.map (Expect.equal ( "could not save the todos: todos.json: permission denied\n", Ok (store False) ))
        ]


saved : String
saved =
    """{ "2": { "title": "milk", "done": false }, "7": { "title": "eggs", "done": true } }"""


store : Bool -> Store
store dirty =
    { todos = Dict.singleton 1 (Todo "milk" False), next = 2, dirty = dirty }


readBack : Nomi.Env -> Task x (Result String Store)
readBack env =
    Main.load env.fs
        |> Task.map Ok
        |> Task.onError (Task.succeed << Err)
