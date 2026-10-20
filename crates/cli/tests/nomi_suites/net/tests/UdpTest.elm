module UdpTest exposing (suite)

import Expect
import Nomi
import Nomi.Bytes
import Nomi.Dns as Dns
import Nomi.Fiber as Fiber
import Nomi.Net as Net exposing (Net)
import Nomi.Test exposing (effect)
import Nomi.Udp as Udp
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Udp and Nomi.Dns"
        [ describe "on a socket" (both (\env -> Task.succeed env.net))
        , describe "on a loopback" (both (\_ -> Net.loopback))
        , effect "localhost goes by 127.0.0.1" <|
            \env ->
                Dns.lookup "localhost" env.net
                    |> Task.mapError Dns.errorToString
                    |> Task.map (Expect.equal True << List.member "127.0.0.1")
        , effect "on a loopback, only this machine's names go by an address" <|
            \_ ->
                Net.loopback
                    |> Task.andThen (\net -> Task.map2 Tuple.pair (Dns.lookup "localhost" net |> Task.mapError Dns.errorToString) (Dns.lookup "example.com" net |> Task.map Ok |> Task.onError (Err >> Task.succeed)))
                    |> Task.map (Expect.equal ( [ "127.0.0.1" ], Err (Dns.NotFound "example.com") ))
        , effect "a narrowed network sends to, and looks up, only the hosts it names" <|
            \env ->
                let
                    narrowed =
                        Net.onlyHosts [ "example.com" ] env.net
                in
                Udp.bind { host = "127.0.0.1", port_ = 0 } narrowed
                    |> Task.andThen (Udp.send { host = "127.0.0.1", port_ = 9 } (Nomi.Bytes.fromString "x"))
                    |> Task.map (\_ -> "sent")
                    |> Task.onError (Udp.errorToString >> Task.succeed)
                    |> Task.andThen (\sent -> Dns.lookup "localhost" narrowed |> Task.map (\_ -> ( sent, "found" )) |> Task.onError (\error -> Task.succeed ( sent, Dns.errorToString error )))
                    |> Task.map (Expect.equal ( "127.0.0.1: not a host the network allows", "localhost: not a host the network allows" ))
        , effect "a network that may not listen binds no port of its own, but a client's" <|
            \env ->
                let
                    closed =
                        Net.noListen env.net
                in
                Task.map2 Tuple.pair
                    (Udp.bind { host = "127.0.0.1", port_ = 9999 } closed |> Task.map (\_ -> "bound") |> Task.onError (Udp.errorToString >> Task.succeed))
                    (Udp.bind { host = "127.0.0.1", port_ = 0 } closed |> Task.map (\_ -> "bound") |> Task.onError (Udp.errorToString >> Task.succeed))
                    |> Task.map (Expect.equal ( "127.0.0.1:9999: permission denied", "bound" ))
        ]


{-| The tests that hold on a socket and on a loopback alike.
-}
both : (Nomi.Env -> Task String Net) -> List Test
both network =
    [ effect "datagrams go both ways, each saying where it came from" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        Task.map2 Tuple.pair (bind net) (bind net)
                            |> Task.andThen
                                (\( a, b ) ->
                                    Udp.send { host = "127.0.0.1", port_ = Udp.localPort b } (Nomi.Bytes.fromString "ping") a
                                        |> Task.andThen (\_ -> Udp.receive b)
                                        |> Task.andThen
                                            (\ping ->
                                                Udp.send { host = ping.host, port_ = ping.port_ } (Nomi.Bytes.fromString "pong") b
                                                    |> Task.andThen (\_ -> Udp.receive a)
                                                    |> Task.map (\pong -> ( text ping, ping.port_ == Udp.localPort a, text pong ))
                                            )
                                        |> Task.mapError Udp.errorToString
                                )
                    )
                |> Task.map (Expect.equal ( "ping", True, "pong" ))
    , effect "the socket of an interrupted fiber closes, freeing its port" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        Fiber.fork (bind net)
                            |> Task.andThen Fiber.join
                            |> Task.andThen (\socket -> Udp.bind { host = "127.0.0.1", port_ = Udp.localPort socket } net |> Task.mapError Udp.errorToString)
                            |> Task.map (\_ -> "bound again")
                    )
                |> Task.map (Expect.equal "bound again")
    ]


bind : Net -> Task String Udp.Socket
bind net =
    Udp.bind { host = "127.0.0.1", port_ = 0 } net |> Task.mapError Udp.errorToString


text : Udp.Datagram -> String
text datagram =
    Nomi.Bytes.toString datagram.data |> Maybe.withDefault "not UTF-8"
