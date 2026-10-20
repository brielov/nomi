module LoopbackTest exposing (suite)

import Expect
import Nomi.Fiber as Fiber
import Nomi.Http.Client as Client
import Nomi.Http.Server as Server
import Nomi.Net as Net exposing (Net)
import Nomi.Queue as Queue
import Nomi.Test exposing (effect)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "a loopback under nomi test"
        [ effect "a handler that sleeps an hour answers at once on the test's clock" <|
            \_ ->
                served (\_ -> Process.sleep 3600000 |> Task.map (\_ -> Server.text 200 "late"))
                    |> Task.andThen (\( net, port_ ) -> Client.sendText (Client.get (url port_)) net |> Task.mapError Client.errorToString)
                    |> Task.andThen (\response -> Time.now |> Task.map (\now -> ( response.body, Time.posixToMillis now )))
                    |> Task.map (Expect.equal ( "late", 3600000 ))
        , effect "a request's timeout passes on the test's clock" <|
            \_ ->
                served (\_ -> Process.sleep 5000 |> Task.map (\_ -> Server.text 200 "late"))
                    |> Task.andThen
                        (\( net, port_ ) ->
                            Client.sendText (Client.get (url port_) |> Client.withTimeout 1000) net
                                |> Task.map (\_ -> "answered")
                                |> Task.onError (Client.errorToString >> Task.succeed)
                        )
                    |> Task.andThen (\outcome -> Time.now |> Task.map (\now -> ( outcome, Time.posixToMillis now )))
                    |> Task.map (Expect.equal ( "the request timed out", 1000 ))
        ]


url : Int -> String
url port_ =
    "http://localhost:" ++ String.fromInt port_ ++ "/"


{-| A loopback, and the port a server of the handler listens on in it.
-}
served : (Server.Request -> Task Never Server.Response) -> Task String ( Net, Int )
served handle =
    let
        defaults =
            Server.defaults
    in
    Task.map2 Tuple.pair Net.loopback Queue.unbounded
        |> Task.andThen
            (\( net, ports ) ->
                Fiber.fork (Server.listen { defaults | port_ = 0, onListen = \port_ -> Queue.offer port_ ports } handle net |> Task.mapError Server.errorToString)
                    |> Task.andThen (\_ -> Queue.take ports)
                    |> Task.map (Tuple.pair net)
            )
