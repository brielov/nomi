module Nomi.Tcp exposing
    ( Connection, connect, connectUnix, reader, writer, peer
    , listen, listenUnix, Config, defaults
    , Error(..), errorToString
    )

{-| TCP connections, and Unix sockets where the system has them, through
the `Net` a program is handed: a connection is its bytes both ways, a
`Stream.Reader` of what the peer sends and a `Stream.Writer` to it.

    echo : Connection -> Task Never ()
    echo connection =
        Nomi.Stream.pipe (Nomi.Tcp.reader connection) (Nomi.Tcp.writer connection)
            |> Task.onError (\_ -> Task.succeed ())

    Nomi.Tcp.listen Nomi.Tcp.defaults echo env.net

A connection is the fiber's that made it, or that a server handed it to:
once that fiber ends, however it ends, the connection closes. Reading
follows the reads: while none waits, the socket isn't read, and the peer
is held back. A write waits until the socket has taken its bytes.

On `Net.loopback`, a server and its clients connect in memory, and a write
is taken at once.

@docs Connection, connect, connectUnix, reader, writer, peer


# Listening

@docs listen, listenUnix, Config, defaults


# Errors

@docs Error, errorToString

-}

import Elm.Kernel.NomiTcp
import Nomi.Net exposing (Net)
import Nomi.Stream as Stream
import Task exposing (Task)



-- CONNECTIONS


{-| A connection to a peer.
-}
type Connection
    = Connection


{-| A connection to the host, by name or address, on the port, which the
network must allow.

    Nomi.Tcp.connect { host = "example.com", port_ = 80 } env.net

-}
connect : { host : String, port_ : Int } -> Net -> Task Error Connection
connect { host, port_ } =
    Elm.Kernel.NomiTcp.connect host port_


{-| A connection to the Unix socket at the path, as a server on this
machine listens on one. A network narrowed with `Nomi.Net.onlyHosts`
reaches none. Windows has no Unix sockets for Bun: there it fails with
`Other`.
-}
connectUnix : String -> Net -> Task Error Connection
connectUnix =
    Elm.Kernel.NomiTcp.connectUnix


{-| What the peer sends, until it ends its side.
-}
reader : Connection -> Stream.Reader
reader =
    Elm.Kernel.NomiTcp.reader


{-| What to send the peer. Closing it ends this side, as a half close does:
the peer reads to its end, and can still send.
-}
writer : Connection -> Stream.Writer
writer =
    Elm.Kernel.NomiTcp.writer


{-| The peer's address, as `"127.0.0.1:52344"`, or the Unix socket's path.
-}
peer : Connection -> String
peer =
    Elm.Kernel.NomiTcp.peer



-- LISTENING


{-| Where to listen, and what to do once the server listens: a `host` of
`"0.0.0.0"` listens on every interface, and a `port_` of 0 on a port the
system picks, which `onListen` is given.
-}
type alias Config =
    { host : String
    , port_ : Int
    , onListen : Int -> Task Never ()
    }


{-| A port the system picks on `127.0.0.1`, doing nothing once it listens.
-}
defaults : Config
defaults =
    { host = "127.0.0.1"
    , port_ = 0
    , onListen = \_ -> Task.succeed ()
    }


{-| Accepts connections until the task is interrupted, handing each to the
function on a fiber of its own, which the connection closes with. Once the
task is interrupted, the server stops accepting, and the fibers of the
connections still open are interrupted.
-}
listen : Config -> (Connection -> Task Never ()) -> Net -> Task Error ()
listen config =
    Elm.Kernel.NomiTcp.listen config.host config.port_ config.onListen


{-| `listen` on a Unix socket at the path, which mustn't exist yet;
`onListen` runs once it listens.
-}
listenUnix : { path : String, onListen : Task Never () } -> (Connection -> Task Never ()) -> Net -> Task Error ()
listenUnix { path, onListen } =
    Elm.Kernel.NomiTcp.listenUnix path onListen



-- ERRORS


{-| Why a connection wasn't made, or a server didn't listen: the network
doesn't allow the host, or listening; no address goes by the host's name;
nothing listens at the address; the address is taken, or the process may
not listen there; or something else, said.
-}
type Error
    = NotAllowed String
    | NotFound String
    | Refused String
    | AddressInUse String
    | PermissionDenied String
    | Other String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        NotAllowed host ->
            host ++ ": not a host the network allows"

        NotFound host ->
            host ++ ": no address goes by this name"

        Refused address ->
            address ++ ": connection refused"

        AddressInUse address ->
            address ++ ": address already in use"

        PermissionDenied address ->
            address ++ ": permission denied"

        Other message ->
            message
