module Nomi.Server exposing
    ( listen, Config, defaults, Error(..), errorToString
    , Request, method, Method(..), path, query, header
    , body, jsonBody, BodyError(..)
    , Response, text, json, empty, withHeader, addHeader
    , request, respond
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

@docs Response, text, json, empty, withHeader, addHeader


# Testing

@docs request, respond

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
Once the server stops accepting, the responses it still writes close their
connections, and a request arriving on one meanwhile is answered 503.

Under `nomi test` a server listens on the test's runtime, whose clock
waits while a request is on the wire: see `Nomi.Test`.

    Nomi.program <|
        \env ->
            Nomi.Server.listen Nomi.Server.defaults handle env.net
                |> Task.mapError Nomi.Server.errorToString

-}
listen : Config -> (Request -> Task Never Response) -> Net -> Task Error ()
listen config handle net =
    Elm.Kernel.NomiServer.listen config (unwrap handle) net


unwrap : (Request -> Task Never Response) -> Request -> Task Never { status : Int, headers : List ( String, String ), body : String }
unwrap handle req =
    Task.map (\(Response response) -> response) (handle req)


{-| Where to listen, how long requests in flight may run on once the
program is asked to stop, the most bytes a request's body may hold, and
what to do once the server listens. A `host` of `"0.0.0.0"` listens on
every interface, and a `port_` of 0 on a port the system picks.

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
    , maxBodyBytes : Int
    , onListen : Int -> Task Never ()
    }


{-| Port 8080 on `127.0.0.1`, draining for 5 seconds, bodies up to 1 MiB,
doing nothing once it listens.
-}
defaults : Config
defaults =
    { port_ = 8080
    , host = "127.0.0.1"
    , drainMilliseconds = 5000
    , maxBodyBytes = 1024 * 1024
    , onListen = \_ -> Task.succeed ()
    }


{-| Why a server could not listen, with the address it was given: the
address is taken, or the process may not listen on it.
-}
type Error
    = AddressInUse { host : String, port_ : Int }
    | PermissionDenied { host : String, port_ : Int }
    | Other String


{-| The error as one line: the address, then what went wrong with it.
-}
errorToString : Error -> String
errorToString error =
    case error of
        AddressInUse address ->
            addressToString address ++ ": address already in use"

        PermissionDenied address ->
            addressToString address ++ ": permission denied"

        Other message ->
            message


addressToString : { host : String, port_ : Int } -> String
addressToString { host, port_ } =
    (if String.contains ":" host then
        "[" ++ host ++ "]"

     else
        host
    )
        ++ ":"
        ++ String.fromInt port_



-- REQUESTS


{-| A request a server received, or one `request` made.
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
method req =
    case Elm.Kernel.NomiServer.method req of
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


{-| The value of the header with the name, in any case. A header sent
more than once is read by Fetch's rules: its values joined with `, `, or
with `; ` for `Cookie`.
-}
header : String -> Request -> Maybe String
header =
    Elm.Kernel.NomiServer.header



-- BODIES


{-| Why a request's body could not be read: it is larger than the
server's `maxBodyBytes`, it is not the JSON expected, with the decoder's
message, or the client went away before sending all of it.
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
jsonBody decoder req =
    body req
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


{-| Adds the header after those the response has, removing any of the
same name, in any case.

    Nomi.Server.text 200 "<p>hi</p>"
        |> Nomi.Server.withHeader "Content-Type" "text/html"

-}
withHeader : String -> String -> Response -> Response
withHeader name value (Response response) =
    let
        other ( existing, _ ) =
            String.toLower existing /= String.toLower name
    in
    Response { response | headers = List.filter other response.headers ++ [ ( name, value ) ] }


{-| Adds the header after those the response has, keeping any of the same
name, for a header sent more than once. Headers go out in the order added,
except that those of one name go together, where the first of them was:

    Nomi.Server.empty 204
        |> Nomi.Server.addHeader "Set-Cookie" "theme=dark"
        |> Nomi.Server.addHeader "Set-Cookie" "lang=en"

-}
addHeader : String -> String -> Response -> Response
addHeader name value (Response response) =
    Response { response | headers = response.headers ++ [ ( name, value ) ] }



-- TESTING


{-| A request with the method, target, headers and body, as a client
would send it, for testing a handler. The target is the path and the
query, and its headers are read as a server reads them:

    Nomi.Server.request
        { method = Post
        , target = "/todos?draft=true"
        , headers = [ ( "Content-Type", "application/json" ) ]
        , body = "{\"title\": \"milk\"}"
        }

-}
request : { method : Method, target : String, headers : List ( String, String ), body : String } -> Request
request r =
    Elm.Kernel.NomiServer.request (methodName r.method) r.target r.headers r.body


methodName : Method -> String
methodName m =
    case m of
        Get ->
            "GET"

        Head ->
            "HEAD"

        Post ->
            "POST"

        Put ->
            "PUT"

        Patch ->
            "PATCH"

        Delete ->
            "DELETE"

        Options ->
            "OPTIONS"

        OtherMethod name ->
            name


{-| Answers the request with the handler as `listen` does, on a fiber of
its own, and gives the response written: a handler that dies answers 500
and logs a line, which a test reads with `Nomi.Test.stderr`, and so does a
response `listen` couldn't write, as status 999 or a line break in a
header. The request's body may hold `defaults`'s
`maxBodyBytes`. The response's headers are as Fetch lists them: names in
lower case and in order, the values of one name joined with `, `, and each
`Set-Cookie` apart. A response with no `Content-Type` and a body gets
`text/plain;charset=utf-8`, as `listen` writes it.

    Nomi.Server.respond handle (Nomi.Server.request { method = Get, target = "/hello/you", headers = [], body = "" })
        |> Task.map (Expect.equal { status = 200, headers = [ ( "content-type", "text/plain; charset=utf-8" ) ], body = "hello, you" })

Interrupting the task is the client disconnecting: the handler is
interrupted and its finalizers run.

-}
respond : (Request -> Task Never Response) -> Request -> Task x { status : Int, headers : List ( String, String ), body : String }
respond handle req =
    Elm.Kernel.NomiServer.answer (unwrap handle) req
