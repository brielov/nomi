module HandlerTest exposing (suite)

import Dict
import Expect
import Main exposing (App)
import Nomi.Fiber as Fiber
import Nomi.Ref as Ref
import Nomi.Server as Server exposing (Method(..))
import Nomi.Test exposing (advance, effect)
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "the handlers"
        [ effect "a todo posted is listed" <|
            \_ ->
                app
                    |> Task.andThen
                        (\a ->
                            Task.map2 Tuple.pair
                                (send a Post "/todos" """{"title": "milk"}""")
                                (send a Get "/todos" "")
                        )
                    |> Task.map
                        (Expect.equal
                            ( json 201 """{"id":1}"""
                            , json 200 """{"1":{"title":"milk","done":false}}"""
                            )
                        )
        , effect "a body that isn't a todo is refused" <|
            \_ ->
                app
                    |> Task.andThen (\a -> send a Post "/todos" "[]")
                    |> Task.map (Expect.equal (text 400 """expected {"title": string}"""))
        , effect "a todo deleted is gone" <|
            \_ ->
                app
                    |> Task.andThen (\a -> send a Post "/todos" """{"title": "milk"}""" |> Task.andThen (\_ -> send a Delete "/todos/1" "") |> Task.andThen (\_ -> send a Get "/todos" ""))
                    |> Task.map (Expect.equal (json 200 "{}"))
        , effect "a todo id that isn't a number is not found" <|
            \_ ->
                app
                    |> Task.andThen (\a -> send a Delete "/todos/milk" "")
                    |> Task.map (Expect.equal (text 404 "no such todo"))
        , effect "a slow quote gives up after two seconds" <|
            \_ ->
                app
                    |> Task.andThen (\a -> Task.map2 Tuple.pair (send a Get "/quote" "") Time.now)
                    |> Task.map (Expect.equal ( text 200 "no quote today", Time.millisToPosix 2000 ))
        , effect "a client that disconnects cancels its request" <|
            \_ ->
                app
                    |> Task.andThen
                        (\a ->
                            Fiber.fork (send a Get "/quote" "")
                                |> Task.andThen (\client -> advance 0 |> Task.andThen (\_ -> send a Get "/health" "") |> Task.map (Tuple.pair client))
                                |> Task.andThen
                                    (\( client, during ) ->
                                        Fiber.interrupt client
                                            |> Task.andThen (\_ -> Task.map2 (\after now -> ( during, after, now )) (send a Get "/health" "") Time.now)
                                    )
                        )
                    |> Task.map
                        (Expect.equal
                            ( json 200 """{"inFlight":1}"""
                            , json 200 """{"inFlight":0}"""
                            , Time.millisToPosix 0
                            )
                        )
        ]


app : Task x App
app =
    Task.map2 App (Ref.make { todos = Dict.empty, next = 1, dirty = False }) (Ref.make 0)


send : App -> Method -> String -> String -> Task x { status : Int, headers : List ( String, String ), body : String }
send a method target body =
    Server.respond (Main.handle a) (Server.request { method = method, target = target, headers = [], body = body })


json : Int -> String -> { status : Int, headers : List ( String, String ), body : String }
json status body =
    { status = status, headers = [ ( "Content-Type", "application/json" ) ], body = body }


text : Int -> String -> { status : Int, headers : List ( String, String ), body : String }
text status body =
    { status = status, headers = [ ( "Content-Type", "text/plain; charset=utf-8" ) ], body = body }
