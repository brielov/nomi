module TlsTest exposing (suite)

import Expect
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Net as Net
import Nomi.Queue as Queue
import Nomi.Stream as Stream
import Nomi.Tcp as Tcp exposing (Connection)
import Nomi.Test exposing (effect)
import Nomi.Tls as Tls
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Tls"
        [ effect "on a loopback, a connection over TLS is one in memory, upgraded or not" <|
            \_ ->
                Net.loopback
                    |> Task.andThen
                        (\net ->
                            Queue.unbounded
                                |> Task.andThen
                                    (\listening ->
                                        Fiber.fork (Tls.listen { certificate = "unused", key = "unused" } { host = "127.0.0.1", port_ = 443, onListen = \_ -> Queue.offer () listening } echo net)
                                            |> Task.andThen (\_ -> Queue.take listening)
                                    )
                                |> Task.andThen (\_ -> Tls.connect Tls.System { host = "127.0.0.1", port_ = 443 } net)
                                |> Task.andThen (Tls.upgrade Tls.System "127.0.0.1")
                                |> Task.mapError Tcp.errorToString
                                |> Task.andThen exchange
                        )
                    |> Task.map (Expect.equal "secret")
        , effect "a narrowed network reaches no host it doesn't name over TLS either" <|
            \env ->
                Tls.connect Tls.System { host = "127.0.0.1", port_ = 443 } (Net.onlyHosts [ "example.com" ] env.net)
                    |> Task.map (\_ -> "connected")
                    |> Task.onError (\error -> Task.succeed (Tcp.errorToString error))
                    |> Task.map (Expect.equal "127.0.0.1: not a host the network allows")
        ]


echo : Connection -> Task Never ()
echo connection =
    Stream.pipe (Tcp.reader connection) (Tcp.writer connection)
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
        |> Task.onError (\_ -> Task.succeed ())


exchange : Connection -> Task String String
exchange connection =
    Stream.write (Nomi.Bytes.fromString "secret") (Tcp.writer connection)
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
        |> Task.andThen (\_ -> Stream.readAll { limit = 100 } (Tcp.reader connection))
        |> Task.map (Nomi.Bytes.toString >> Maybe.withDefault "not UTF-8")
        |> Task.mapError Stream.errorToString
