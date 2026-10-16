module Main exposing (Store, Todo, flush, flushEvery, load, main)

{-| A small todo API: a JSON file for storage, a `Ref` for the live state,
a background fiber that writes the file every five seconds when it
changed, a timeout on a slow dependency, and a graceful shutdown that
lets requests in flight finish and writes the file once more.
-}

import Dict exposing (Dict)
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Nomi
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Ref as Ref exposing (Ref)
import Nomi.Server as Server exposing (Method(..), Request, Response)
import Nomi.Task
import Process
import Task exposing (Task)


type alias Todo =
    { title : String, done : Bool }


type alias Store =
    { todos : Dict Int Todo, next : Int, dirty : Bool }


{-| The live state, and how many requests are being answered.
-}
type alias App =
    { store : Ref Store, inFlight : Ref Int }


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                data =
                    Fs.subdir "data" env.fs

                defaults =
                    Server.defaults

                config =
                    { defaults
                        | port_ = Dict.get "PORT" env.vars |> Maybe.andThen String.toInt |> Maybe.withDefault defaults.port_
                        , onListen = \port_ -> Nomi.log ("listening on " ++ String.fromInt port_)
                    }
            in
            Fs.makeDirectory "data" env.fs
                |> Task.mapError Fs.errorToString
                |> Task.andThen (\_ -> load data)
                |> Task.andThen (\store -> Task.map2 App (Ref.make store) (Ref.make 0))
                |> Task.andThen
                    (\app ->
                        Nomi.Task.acquireRelease
                            (Fiber.fork (flushEvery 5000 data app.store))
                            (\flusher -> Fiber.interrupt flusher |> Task.andThen (\_ -> flush data app.store))
                            (\_ -> Server.listen config (handle app) env.net |> Task.mapError Server.errorToString)
                    )


{-| The todos saved in `todos.json`, or none if there is no such file.
-}
load : Fs -> Task String Store
load fs =
    Fs.read "todos.json" fs
        |> Task.onError
            (\error ->
                case error of
                    Fs.NotFound _ ->
                        Task.succeed "{}"

                    _ ->
                        Task.fail error
            )
        |> Task.mapError Fs.errorToString
        |> Task.andThen (Decode.decodeString decoder >> Result.mapError (\error -> "todos.json: " ++ Decode.errorToString error) >> Nomi.Task.fromResult)
        |> Task.map (\todos -> { todos = todos, next = nextId todos, dirty = False })


{-| Flushes the store every `ms` milliseconds.
-}
flushEvery : Float -> Fs -> Ref Store -> Task x ()
flushEvery ms fs store =
    Process.sleep ms
        |> Task.andThen (\_ -> flush fs store)
        |> Task.andThen (\_ -> flushEvery ms fs store)


{-| Writes the todos to `todos.json` if they changed since the last flush.
A write that fails is logged, and tried again by the next one.
-}
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
                            |> Task.onError
                                (\error ->
                                    Ref.update (\now -> { now | dirty = True }) store
                                        |> Task.andThen (\_ -> Nomi.log ("could not save the todos: " ++ Fs.errorToString error))
                                )

                    else
                        Task.succeed ()
                )
        )


handle : App -> Request -> Task Never Response
handle app req =
    case ( Server.method req, Server.path req ) of
        ( Get, [ "health" ] ) ->
            Ref.get app.inFlight
                |> Task.map (\n -> Server.json 200 (Encode.object [ ( "inFlight", Encode.int n ) ]))

        _ ->
            -- Counted until it ends, however it ends: a client that
            -- disconnects interrupts it, and the release still runs.
            Nomi.Task.acquireRelease
                (Ref.update ((+) 1) app.inFlight)
                (\_ -> Ref.update (\n -> n - 1) app.inFlight)
                (\_ -> route app.store req)


route : Ref Store -> Request -> Task Never Response
route store req =
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
