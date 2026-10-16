module Main exposing (Settings, handle, main, seed, settings)

{-| The todo api: `GET /todos` lists the todos and `POST /todos` adds one,
both as JSON written by acme/shared's codecs. The web app is served from
another origin, so every response says that origin may read it.
-}

import Json.Encode as Encode
import Nomi
import Nomi.Ref as Ref exposing (Ref)
import Nomi.Server as Server exposing (Method(..), Request, Response)
import Task exposing (Task)
import Todo exposing (Todo)


{-| Where to listen, and the origin of the page that calls the api.
-}
type alias Settings =
    { port_ : Int, origin : String }


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            let
                given =
                    settings env.args

                defaults =
                    Server.defaults

                config =
                    { defaults
                        | port_ = given.port_
                        , onListen = \port_ -> Nomi.log ("listening on http://localhost:" ++ String.fromInt port_)
                    }
            in
            Ref.make seed
                |> Task.andThen
                    (\todos ->
                        Server.listen config (handle given.origin todos) env.net
                            |> Task.mapError Server.errorToString
                    )


{-| The settings from `--port` and `--origin`, which nomi.toml's `args` give.
-}
settings : List String -> Settings
settings args =
    case args of
        "--port" :: value :: rest ->
            let
                given =
                    settings rest
            in
            { given | port_ = String.toInt value |> Maybe.withDefault given.port_ }

        "--origin" :: value :: rest ->
            let
                given =
                    settings rest
            in
            { given | origin = value }

        _ :: rest ->
            settings rest

        [] ->
            { port_ = 3000, origin = "http://localhost:8000" }


{-| The todos the api starts with.
-}
seed : List Todo
seed =
    [ Todo 1 "Edit packages/shared and watch both apps pick it up" False ]


{-| Answers a request, letting the page at `origin` read the response.
-}
handle : String -> Ref (List Todo) -> Request -> Task Never Response
handle origin todos req =
    route todos req
        |> Task.map (Server.withHeader "Access-Control-Allow-Origin" origin)


route : Ref (List Todo) -> Request -> Task Never Response
route todos req =
    case ( Server.method req, Server.path req ) of
        ( Options, _ ) ->
            -- The browser asks before it posts JSON to another origin.
            Server.empty 204
                |> Server.withHeader "Access-Control-Allow-Methods" "GET, POST"
                |> Server.withHeader "Access-Control-Allow-Headers" "Content-Type"
                |> Task.succeed

        ( Get, [ "todos" ] ) ->
            Ref.get todos
                |> Task.map (\all -> Server.json 200 (Encode.list Todo.encode all))

        ( Post, [ "todos" ] ) ->
            Server.jsonBody Todo.titleDecoder req
                |> Task.andThen (\title -> Ref.modify (add title) todos)
                |> Task.map (\todo -> Server.json 201 (Todo.encode todo))
                |> Task.onError (\_ -> Task.succeed (Server.text 400 "expected {\"title\": string}"))

        _ ->
            Task.succeed (Server.text 404 "not found")


add : String -> List Todo -> ( Todo, List Todo )
add title all =
    let
        todo =
            Todo (List.length all + 1) title False
    in
    ( todo, all ++ [ todo ] )
