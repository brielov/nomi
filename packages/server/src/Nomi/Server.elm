module Nomi.Server exposing
    ( listen, Config, defaults, Error(..), errorToString
    , Request, method, Method(..), path, query, header
    , body, jsonBody, BodyError(..)
    , Response, text, json, empty, withHeader
    )

{-| HTTP servers. A handler is a function from a request to the task of
its response; `case ( method req, path req ) of` routes:

    handle : Request -> Task Never Response
    handle req =
        case ( Nomi.Server.method req, Nomi.Server.path req ) of
            ( Get, [ "hello", name ] ) ->
                Task.succeed (Nomi.Server.text 200 ("hello, " ++ name))

            _ ->
                Task.succeed (Nomi.Server.text 404 "not found")


# Listening

@docs listen, Config, defaults, Error, errorToString


# Requests

@docs Request, method, Method, path, query, header


# Bodies

@docs body, jsonBody, BodyError


# Responses

@docs Response, text, json, empty, withHeader

-}

import Elm.Kernel.NomiServer
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Nomi exposing (Net)
import Task exposing (Task)



-- LISTENING


{-| Serves HTTP on the configured host and port, answering each request
with the handler's response, until the task is interrupted. Then the
server stops accepting connections, gives the requests in flight
`drainMilliseconds` to finish, interrupts the rest, and closes their
connections.

Each request runs on a fiber of its own. A client that disconnects before
its response is written interrupts it, so its finalizers run; a handler
that dies, as `Debug.todo` does, answers 500 and writes a line to stderr.

    Nomi.program <|
        \env ->
            Nomi.Server.listen Nomi.Server.defaults handle env.net
                |> Task.mapError Nomi.Server.errorToString

-}
listen : Config -> (Request -> Task Never Response) -> Net -> Task Error ()
listen config handle net =
    Elm.Kernel.NomiServer.listen config (\request -> Task.map (\(Response response) -> response) (handle request)) net


{-| Where to listen, how long requests in flight may run on once the
program is asked to stop, and what to do once the server listens. A
`host` of `"0.0.0.0"` listens on every interface, and a `port_` of 0 on a
port the system picks.

`onListen` runs with the port the server took, once it is bound and
before it answers a request:

    let
        defaults =
            Nomi.Server.defaults
    in
    Nomi.Server.listen
        { defaults
            | port_ = 0
            , onListen = \port_ -> Nomi.log ("listening on " ++ String.fromInt port_)
        }
        handle
        env.net

-}
type alias Config =
    { port_ : Int
    , host : String
    , drainMilliseconds : Int
    , onListen : Int -> Task Never ()
    }


{-| Port 8080 on `127.0.0.1`, draining for 5 seconds, doing nothing once it
listens.
-}
defaults : Config
defaults =
    { port_ = 8080
    , host = "127.0.0.1"
    , drainMilliseconds = 5000
    , onListen = \_ -> Task.succeed ()
    }


{-| Why a server could not listen: the port is taken, or the process may
not listen on it.
-}
type Error
    = AddressInUse
    | PermissionDenied
    | OtherError String


{-| The error as a sentence.
-}
errorToString : Error -> String
errorToString error =
    case error of
        AddressInUse ->
            "the address is already in use"

        PermissionDenied ->
            "permission to listen on the address was denied"

        OtherError message ->
            message



-- REQUESTS


{-| A request a server received.
-}
type Request
    = Request


{-| A request's method.
-}
type Method
    = Get
    | Head
    | Post
    | Put
    | Patch
    | Delete
    | Options
    | OtherMethod String


{-| The request's method.
-}
method : Request -> Method
method request =
    case Elm.Kernel.NomiServer.method request of
        "GET" ->
            Get

        "HEAD" ->
            Head

        "POST" ->
            Post

        "PUT" ->
            Put

        "PATCH" ->
            Patch

        "DELETE" ->
            Delete

        "OPTIONS" ->
            Options

        other ->
            OtherMethod other


{-| The request's path, as its percent-decoded segments, without empty
ones: `/todos/3` and `/todos/3/` are `[ "todos", "3" ]`, and `/` is `[]`.
-}
path : Request -> List String
path =
    Elm.Kernel.NomiServer.path


{-| The first value of the query parameter with the name, percent-decoded:
`/todos?done=true` has `query "done"` of `Just "true"`.
-}
query : String -> Request -> Maybe String
query =
    Elm.Kernel.NomiServer.query


{-| The value of the header with the name, in any case. Repeated headers
are joined with commas.
-}
header : String -> Request -> Maybe String
header =
    Elm.Kernel.NomiServer.header



-- BODIES


{-| Why a request's body could not be read: it is larger than 1 MiB, it is
not the JSON expected, with the decoder's message, or the client went away
before sending all of it.
-}
type BodyError
    = TooLarge
    | BadJson String
    | Disconnected


{-| The request's body, as UTF-8 text. It is read once, however many
times it is asked for.
-}
body : Request -> Task BodyError String
body =
    Elm.Kernel.NomiServer.body


{-| The request's body, decoded as JSON.

    Nomi.Server.jsonBody (Decode.field "title" Decode.string) req

-}
jsonBody : Decoder a -> Request -> Task BodyError a
jsonBody decoder request =
    body request
        |> Task.andThen
            (\text_ ->
                case Decode.decodeString decoder text_ of
                    Ok value ->
                        Task.succeed value

                    Err error ->
                        Task.fail (BadJson (Decode.errorToString error))
            )



-- RESPONSES


{-| A response to a request.
-}
type Response
    = Response
        { status : Int
        , headers : List ( String, String )
        , body : String
        }


{-| A plain text response with the status.
-}
text : Int -> String -> Response
text status content =
    Response { status = status, headers = [ ( "Content-Type", "text/plain; charset=utf-8" ) ], body = content }


{-| A JSON response with the status.
-}
json : Int -> Encode.Value -> Response
json status value =
    Response { status = status, headers = [ ( "Content-Type", "application/json" ) ], body = Encode.encode 0 value }


{-| A response with the status and no body.
-}
empty : Int -> Response
empty status =
    Response { status = status, headers = [], body = "" }


{-| Adds the header, replacing any of the same name, in any case.

    Nomi.Server.text 200 "<p>hi</p>"
        |> Nomi.Server.withHeader "Content-Type" "text/html"

-}
withHeader : String -> String -> Response -> Response
withHeader name value (Response response) =
    let
        other ( existing, _ ) =
            String.toLower existing /= String.toLower name
    in
    Response { response | headers = ( name, value ) :: List.filter other response.headers }
