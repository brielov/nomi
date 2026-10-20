module Nomi.Http.WebSocket exposing
    ( WebSocket, Message(..), receive, send, close, closed
    , connect
    , Error(..), errorToString
    )

{-| WebSockets: messages both ways on one connection, which a server
makes of a request with `Nomi.Http.Server.upgrade`, and a client with
`connect`.

    echo : WebSocket -> Task Error ()
    echo socket =
        Nomi.Http.WebSocket.receive socket
            |> Task.andThen (\message -> Nomi.Http.WebSocket.send message socket)
            |> Task.andThen (\_ -> echo socket)


# Connections

@docs WebSocket, Message, receive, send, close, closed


# Clients

@docs connect


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiWebSocket
import Nomi.Net exposing (Net)
import Task exposing (Task)


{-| A connection to a peer.
-}
type WebSocket
    = WebSocket


{-| A message, of text or of bytes.
-}
type Message
    = Text String
    | Binary Bytes


{-| The next message the peer sent, oldest first, waiting until one comes.
Once the connection closed, and every message it received is taken, it
fails with `Closed`.
-}
receive : WebSocket -> Task Error Message
receive =
    Elm.Kernel.NomiWebSocket.receive


{-| Sends the message, succeeding once it is on its way. When the peer
takes messages more slowly than they are sent, a send waits until what
was sent before drained, so a slow peer holds back whatever produces the
messages. On a closed connection it fails with `Closed`.
-}
send : Message -> WebSocket -> Task Error ()
send =
    Elm.Kernel.NomiWebSocket.send


{-| Closes the connection with the code, as 1000 for a normal close or one
from 4000 to 4999 of the application's own, and the reason. Closing a
closed connection does nothing.
-}
close : Int -> String -> WebSocket -> Task x ()
close =
    Elm.Kernel.NomiWebSocket.close


{-| How the connection closed, once it did, with the code and reason the
close gave, as a finalizer reads it once the peer left.
-}
closed : WebSocket -> Task x (Maybe { code : Int, reason : String })
closed =
    Elm.Kernel.NomiWebSocket.closedWith


{-| Connects to the URL, `ws://` or `wss://`, on the network. The
connection is the fiber's that made it: once that fiber ends, it is
closed with 1000. On `Nomi.Net.loopback`, a server listening there
answers.

    Nomi.Http.WebSocket.connect "ws://localhost:8080/chat" env.net

-}
connect : String -> Net -> Task Error WebSocket
connect =
    Elm.Kernel.NomiWebSocket.connect


{-| What went wrong: the connection closed, with the code and reason the
close gave; or connecting failed, the URL not being one, the network not
allowing its host, the host not answering, or its server answering
without upgrading the request.
-}
type Error
    = Closed { code : Int, reason : String }
    | BadUrl String
    | NotAllowed String
    | Unreachable String
    | Refused


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        Closed { code, reason } ->
            "the connection closed with " ++ String.fromInt code ++ (if reason == "" then "" else ": " ++ reason)

        BadUrl url ->
            url ++ ": not a WebSocket URL"

        NotAllowed host ->
            host ++ ": not a host the network allows"

        Unreachable why ->
            why

        Refused ->
            "the server answered without upgrading the request"
