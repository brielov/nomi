module Nomi.Udp exposing
    ( Socket, bind, localPort, send, receive, Datagram
    , Error(..), errorToString
    )

{-| UDP: datagrams, each sent on its own, which may arrive out of order,
twice, or not at all.

    Nomi.Udp.bind { host = "0.0.0.0", port_ = 0 } env.net
        |> Task.andThen (\socket -> Nomi.Udp.send { host = "127.0.0.1", port_ = 9999 } (Nomi.Bytes.fromString "ping") socket)

A socket is the fiber's that bound it, and closes once that fiber ends. It
holds the 1,000 datagrams no `receive` has taken; past them, more are
dropped, as a full socket's buffer drops them. On `Net.loopback`, sockets
send to each other in memory.

@docs Socket, bind, localPort, send, receive, Datagram


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiUdp
import Nomi.Net exposing (Net)
import Task exposing (Task)


{-| A socket bound to a port.
-}
type Socket
    = Socket


{-| A socket on the host's interface and the port: `"0.0.0.0"` binds every
interface, and port 0 one the system picks, as a client's is. Binding a
port of its own is listening, which the network must allow.
-}
bind : { host : String, port_ : Int } -> Net -> Task Error Socket
bind { host, port_ } =
    Elm.Kernel.NomiUdp.bind host port_


{-| The port the socket is bound to.
-}
localPort : Socket -> Int
localPort =
    Elm.Kernel.NomiUdp.localPort


{-| Sends the bytes to the host, by address or name, and port, which the
network must allow. It succeeds once the socket has taken them, which
says nothing of their arriving.
-}
send : { host : String, port_ : Int } -> Bytes -> Socket -> Task Error ()
send { host, port_ } =
    Elm.Kernel.NomiUdp.send host port_


{-| A datagram received, and where it came from.
-}
type alias Datagram =
    { data : Bytes
    , host : String
    , port_ : Int
    }


{-| The next datagram, once one comes.
-}
receive : Socket -> Task Error Datagram
receive =
    Elm.Kernel.NomiUdp.receive


{-| Why a socket wasn't bound or a datagram sent: the network doesn't allow
the host or listening; the port is taken, or the process may not bind it;
the socket is closed; or something else, said.
-}
type Error
    = NotAllowed String
    | AddressInUse String
    | PermissionDenied String
    | Closed
    | Other String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        NotAllowed host ->
            host ++ ": not a host the network allows"

        AddressInUse address ->
            address ++ ": address already in use"

        PermissionDenied address ->
            address ++ ": permission denied"

        Closed ->
            "the socket is closed"

        Other message ->
            message
