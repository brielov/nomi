module HandlerTest exposing (suite)

import Expect
import Main
import Nomi.Ref as Ref
import Nomi.Http.Server as Server exposing (Method(..))
import Nomi.Test exposing (effect)
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "the api"
        [ effect "lists the todos" <|
            \_ ->
                send Get "/todos" ""
                    |> Task.map
                        (Expect.equal
                            (json 200 """[{"id":1,"title":"Edit packages/shared and watch both apps pick it up","done":false}]""")
                        )
        , effect "adds a todo, which is then listed" <|
            \_ ->
                api
                    |> Task.andThen
                        (\handle ->
                            Task.map2 Tuple.pair
                                (Server.respond handle (request Post "/todos" """{"title": "milk"}"""))
                                (Server.respond handle (request Get "/todos" ""))
                        )
                    |> Task.map
                        (Tuple.mapSecond .body
                            >> Expect.equal
                                ( json 201 """{"id":2,"title":"milk","done":false}"""
                                , """[{"id":1,"title":"Edit packages/shared and watch both apps pick it up","done":false},{"id":2,"title":"milk","done":false}]"""
                                )
                        )
        , effect "refuses a body without a title" <|
            \_ ->
                send Post "/todos" "{}"
                    |> Task.map
                        (Expect.equal
                            { status = 400
                            , headers = [ cors, ( "content-type", "text/plain; charset=utf-8" ) ]
                            , body = """expected {"title": string}"""
                            }
                        )
        , effect "lets the page post JSON from its origin" <|
            \_ ->
                send Options "/todos" ""
                    |> Task.map
                        (Expect.equal
                            { status = 204
                            , headers =
                                [ ( "access-control-allow-headers", "Content-Type" )
                                , ( "access-control-allow-methods", "GET, POST" )
                                , cors
                                ]
                            , body = ""
                            }
                        )
        , Test.test "takes its port and the page's origin from its arguments" <|
            \_ ->
                Main.settings [ "--port", "3001", "--origin", "http://localhost:8080" ]
                    |> Expect.equal { port_ = 3001, origin = "http://localhost:8080" }
        ]


{-| The api's handler, on the todos it starts with.
-}
api : Task x (Server.Request -> Task Never Server.Response)
api =
    Ref.make Main.seed |> Task.map (Main.handle "http://localhost:8000")


send : Method -> String -> String -> Task x { status : Int, headers : List ( String, String ), body : String }
send method target body =
    api |> Task.andThen (\handle -> Server.respond handle (request method target body))


request : Method -> String -> String -> Server.Request
request method target body =
    Server.request { method = method, target = target, headers = [], body = body }


cors : ( String, String )
cors =
    ( "access-control-allow-origin", "http://localhost:8000" )


json : Int -> String -> { status : Int, headers : List ( String, String ), body : String }
json status body =
    { status = status, headers = [ cors, ( "content-type", "application/json" ) ], body = body }
