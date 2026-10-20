module TcpTest exposing (suite)

import Expect
import Nomi
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Net as Net exposing (Net)
import Nomi.Queue as Queue exposing (Queue)
import Nomi.Stream as Stream
import Nomi.Tcp as Tcp exposing (Connection)
import Nomi.Test exposing (effect)
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Tcp"
        [ describe "on a socket" (both (\env -> Task.succeed env.net))
        , describe "on a loopback" (both (\_ -> Net.loopback))
        , effect "on a loopback, a Unix socket echoes as a port does" <|
            \_ ->
                Net.loopback
                    |> Task.andThen
                        (\net ->
                            Queue.unbounded
                                |> Task.andThen (\listening -> Fiber.fork (Tcp.listenUnix { path = "app.sock", onListen = Queue.offer () listening } echo net) |> Task.andThen (\_ -> Queue.take listening))
                                |> Task.andThen (\_ -> Tcp.connectUnix "app.sock" net |> Task.mapError Tcp.errorToString)
                                |> Task.andThen (\connection -> exchange [ "over ", "unix" ] connection |> Task.map (\reply -> ( reply, Tcp.peer connection )))
                        )
                    |> Task.map (Expect.equal ( "over unix", "app.sock" ))
        , effect "a narrowed network reaches neither a host it doesn't name nor a Unix socket" <|
            \env ->
                let
                    narrowed =
                        Net.onlyHosts [ "example.com" ] env.net
                in
                Task.map2 Tuple.pair
                    (Tcp.connect { host = "127.0.0.1", port_ = 9 } narrowed |> outcome)
                    (Tcp.connectUnix "nomi.sock" narrowed |> outcome)
                    |> Task.map (Expect.equal ( "Err (NotAllowed \"127.0.0.1\")", "Err (NotAllowed \"nomi.sock\")" ))
        , effect "a network that may not listen refuses a server" <|
            \env ->
                Tcp.listen Tcp.defaults (\_ -> Task.succeed ()) (Net.noListen env.net)
                    |> outcome
                    |> Task.map (Expect.equal "Err (PermissionDenied \"127.0.0.1:0\")")
        , effect "a port taken is in use" <|
            \env ->
                serving env.net echo
                    |> Task.andThen (\port_ -> Tcp.listen { host = "127.0.0.1", port_ = port_, onListen = \_ -> Task.succeed () } echo env.net |> outcome |> Task.map (String.replace (String.fromInt port_) "port"))
                    |> Task.map (Expect.equal "Err (AddressInUse \"127.0.0.1:port\")")
        ]


{-| The tests that hold on a socket and on a loopback alike.
-}
both : (Nomi.Env -> Task String Net) -> List Test
both network =
    [ effect "a server echoes what a client sends, to the end the client sets" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        serving net echo
                            |> Task.andThen (\port_ -> Tcp.connect { host = "127.0.0.1", port_ = port_ } net |> Task.mapError Tcp.errorToString)
                            |> Task.andThen (exchange [ "hello, ", "world" ])
                    )
                |> Task.map (Expect.equal "hello, world")
    , effect "a connection to a port nothing listens on is refused" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        Queue.unbounded
                            |> Task.andThen
                                (\listening ->
                                    Fiber.fork (Tcp.listen { host = "127.0.0.1", port_ = 0, onListen = \port_ -> Queue.offer port_ listening } echo net)
                                        |> Task.andThen (\server -> Queue.take listening |> Task.andThen (\port_ -> Fiber.interrupt server |> Task.map (\_ -> port_)))
                                )
                            |> Task.andThen (\port_ -> Tcp.connect { host = "127.0.0.1", port_ = port_ } net |> outcome |> Task.map (String.replace (String.fromInt port_) "port"))
                    )
                |> Task.map (Expect.equal "Err (Refused \"127.0.0.1:port\")")
    , effect "the socket of an interrupted fiber closes, and its peer reads to the end" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        Queue.unbounded
                            |> Task.andThen
                                (\heard ->
                                    serving net
                                        (\connection ->
                                            Queue.offer (Ok "accepted") heard
                                                |> Task.andThen (\_ -> Stream.read (Tcp.reader connection) |> Task.map (Debug.toString >> Ok) |> Task.onError (Debug.toString >> Err >> Task.succeed))
                                                |> Task.andThen (\read -> Queue.offer read heard)
                                        )
                                        |> Task.andThen
                                            (\port_ ->
                                                Fiber.fork (Tcp.connect { host = "127.0.0.1", port_ = port_ } net |> Task.andThen (\connection -> Stream.read (Tcp.reader connection) |> Task.mapError (\_ -> Tcp.Other "read")))
                                                    |> Task.andThen (\client -> Queue.take heard |> Task.andThen (\accepted -> Fiber.interrupt client |> Task.map (\_ -> accepted)))
                                                    |> Task.andThen (\accepted -> Queue.take heard |> Task.map (Tuple.pair accepted))
                                            )
                                )
                    )
                |> Task.map (Expect.equal ( Ok "accepted", Ok "Nothing" ))
    , effect "each connection is a fiber of its own" <|
        \env ->
            network env
                |> Task.andThen
                    (\net ->
                        serving net echo
                            |> Task.andThen
                                (\port_ ->
                                    List.map (\word -> Tcp.connect { host = "127.0.0.1", port_ = port_ } net |> Task.mapError Tcp.errorToString |> Task.andThen (exchange [ word ])) [ "one", "two", "three" ]
                                        |> Task.sequence
                                )
                    )
                |> Task.map (Expect.equal [ "one", "two", "three" ])
    ]


{-| Listens with `handle` on a port the system picks, on a fiber of the
test's, and gives the port once the server listens.
-}
serving : Net -> (Connection -> Task Never ()) -> Task String Int
serving net handle =
    Queue.unbounded
        |> Task.andThen
            (\listening ->
                Fiber.fork (Tcp.listen { host = "127.0.0.1", port_ = 0, onListen = \port_ -> Queue.offer port_ listening } handle net)
                    |> Task.andThen (\_ -> Queue.take listening)
            )


echo : Connection -> Task Never ()
echo connection =
    Stream.pipe (Tcp.reader connection) (Tcp.writer connection)
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
        |> Task.onError (\_ -> Task.succeed ())


{-| Sends the words, ends this side, and reads what comes back to its end.
-}
exchange : List String -> Connection -> Task String String
exchange words connection =
    List.map (\word -> Stream.write (Nomi.Bytes.fromString word) (Tcp.writer connection)) words
        |> Task.sequence
        |> Task.andThen (\_ -> Stream.close (Tcp.writer connection))
        |> Task.andThen (\_ -> Stream.readAll { limit = 1024 } (Tcp.reader connection))
        |> Task.map (Nomi.Bytes.toString >> Maybe.withDefault "not UTF-8")
        |> Task.mapError Stream.errorToString


outcome : Task Tcp.Error a -> Task x String
outcome task =
    task
        |> Task.map (\_ -> "Ok")
        |> Task.onError (\error -> Task.succeed (Debug.toString (Err error)))
