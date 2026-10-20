module Nomi.Http.Server exposing
    ( listen, Config, defaults, Error(..), errorToString
    , Request, method, Method(..), path, query, header
    , body, bodyBytes, bodyText, bodyJson, BodyError(..)
    , Response, text, json, bytes, empty, stream, file, withHeader, addHeader
    , events, event
    , request, respond
    )

{-| HTTP servers. A handler is a function from a request to the task of
its response; `case ( method req, path req ) of` routes:

    handle : Request -> Task Never Response
    handle req =
        case ( Nomi.Http.Server.method req, Nomi.Http.Server.path req ) of
            ( Get, [ "hello", name ] ) ->
                Task.succeed (Nomi.Http.Server.text 200 ("hello, " ++ name))

            _ ->
                Task.succeed (Nomi.Http.Server.text 404 "not found")


# Listening

@docs listen, Config, defaults, Error, errorToString


# Requests

@docs Request, method, Method, path, query, header


# Bodies

A request's body is a stream of bytes, read as it arrives, which fails
with `Nomi.Stream.TooLarge` past the server's `maxBodyBytes`. The helpers
read it whole, so a body is read once: after that it is empty.

@docs body, bodyBytes, bodyText, bodyJson, BodyError


# Responses

@docs Response, text, json, bytes, empty, stream, file, withHeader, addHeader


# Server-sent events

@docs events, event


# Testing

@docs request, respond

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiHttpServer
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Nomi.Net exposing (Net)
import Nomi.Bytes
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Path
import Nomi.Stream as Stream
import Task exposing (Task)



-- LISTENING


{-| Serves HTTP on the configured host and port, answering each request
with the handler's response, until the task is interrupted. Then the
server stops accepting connections, gives the requests in flight
`drainMilliseconds` to finish, interrupts the rest, and closes their
connections.

Each request runs on a fiber of its own, which also writes a streamed
response's body, so what the handler opened for it, as a `file`, stays
open until it is sent. A client that disconnects before its response is
written interrupts it, so its finalizers run; a handler that dies, as
`Debug.todo` does, answers 500 and writes a line to stderr. Once the
server stops accepting, the responses it still writes close their
connections, and a request arriving on one meanwhile is answered 503.

Under `nomi test` a server listens on the test's runtime, whose clock
waits while a request is on the wire: see `Nomi.Test`.

    Nomi.program <|
        \env ->
            Nomi.Http.Server.listen Nomi.Http.Server.defaults handle env.net
                |> Task.mapError Nomi.Http.Server.errorToString

-}
listen : Config -> (Request -> Task Never Response) -> Net -> Task Error ()
listen config handle net =
    Elm.Kernel.NomiHttpServer.listen config (unwrap handle) net


{-| A response as the kernel takes it.
-}
unwrap :
    (Request -> Task Never Response)
    -> Request
    -> Task Never { status : Int, headers : List ( String, String ), text : Maybe String, bytes : Maybe Bytes, reader : Maybe Stream.Reader, opening : Maybe String, file : Maybe File }
unwrap handle req =
    handle req
        |> Task.map
            (\(Response response) ->
                let
                    none =
                        { status = response.status, headers = response.headers, text = Nothing, bytes = Nothing, reader = Nothing, opening = Nothing, file = Nothing }
                in
                case response.body of
                    NoBody ->
                        none

                    TextBody content ->
                        { none | text = Just content }

                    BytesBody content ->
                        { none | bytes = Just content }

                    StreamBody content ->
                        { none | reader = Just content }

                    EventsBody content ->
                        { none | reader = Just content, opening = Just ":\n\n" }

                    FileBody content ->
                        { none | file = Just content }
            )


{-| Where to listen, how long requests in flight may run on once the
program is asked to stop, the most bytes a request's body may hold, the
certificate and key to serve HTTPS with, in PEM, and what to do once the
server listens. A `host` of `"0.0.0.0"` listens on every interface, and a
`port_` of 0 on a port the system picks.

`onListen` runs with the port the server took, once it is bound and
before it answers a request:

    let
        defaults =
            Nomi.Http.Server.defaults
    in
    Nomi.Http.Server.listen
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
    , tls : Maybe { certificate : String, key : String }
    , onListen : Int -> Task Never ()
    }


{-| Port 8080 on `127.0.0.1`, plain HTTP, draining for 5 seconds, bodies
up to 1 MiB, doing nothing once it listens.
-}
defaults : Config
defaults =
    { port_ = 8080
    , host = "127.0.0.1"
    , drainMilliseconds = 5000
    , maxBodyBytes = 1024 * 1024
    , tls = Nothing
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
    case Elm.Kernel.NomiHttpServer.method req of
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
    Elm.Kernel.NomiHttpServer.path


{-| The first value of the query parameter with the name, percent-decoded:
`/todos?done=true` has `query "done"` of `Just "true"`.
-}
query : String -> Request -> Maybe String
query =
    Elm.Kernel.NomiHttpServer.query


{-| The value of the header with the name, in any case. A header sent
more than once is read by Fetch's rules: its values joined with `, `, or
with `; ` for `Cookie`.
-}
header : String -> Request -> Maybe String
header =
    Elm.Kernel.NomiHttpServer.header



-- BODIES


{-| The request's body, as it arrives: a reader that fails with
`Nomi.Stream.TooLarge` once it gave more than the server's `maxBodyBytes`,
and with `Nomi.Stream.Closed` when the client goes away first. A body
whose `Content-Length` says it is too large fails before anything is
read.

    upload : Fs -> Request -> Task Never Response
    upload fs req =
        Nomi.Fs.writer "upload.bin" fs
            |> Task.mapError (\_ -> Nomi.Stream.Other "can't write")
            |> Task.andThen (Nomi.Stream.pipe (Nomi.Http.Server.body req))
            |> Task.map (\_ -> Nomi.Http.Server.empty 204)
            |> Task.onError (\_ -> Task.succeed (Nomi.Http.Server.empty 400))

-}
body : Request -> Stream.Reader
body =
    Elm.Kernel.NomiHttpServer.body


{-| Why a request's body could not be read: it is larger than the
server's `maxBodyBytes`, the client went away before sending all of it, it
is not UTF-8 text, or it is not the JSON expected, with the decoder's
message.
-}
type BodyError
    = TooLarge
    | Disconnected
    | NotUtf8
    | BadJson String


{-| The request's body, whole.
-}
bodyBytes : Request -> Task BodyError Bytes
bodyBytes req =
    Stream.readAll { limit = Elm.Kernel.NomiHttpServer.limit req } (body req)
        |> Task.mapError
            (\error ->
                case error of
                    Stream.TooLarge ->
                        TooLarge

                    _ ->
                        Disconnected
            )


{-| The request's body, as UTF-8 text, refusing one that isn't
(`NotUtf8`).
-}
bodyText : Request -> Task BodyError String
bodyText req =
    bodyBytes req
        |> Task.andThen
            (\all ->
                case Nomi.Bytes.toString all of
                    Just content ->
                        Task.succeed content

                    Nothing ->
                        Task.fail NotUtf8
            )


{-| The request's body, decoded as JSON.

    Nomi.Http.Server.bodyJson (Decode.field "title" Decode.string) req

-}
bodyJson : Decoder a -> Request -> Task BodyError a
bodyJson decoder req =
    bodyText req
        |> Task.andThen
            (\content ->
                case Decode.decodeString decoder content of
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
        , body : Body
        }


type Body
    = NoBody
    | TextBody String
    | BytesBody Bytes
    | StreamBody Stream.Reader
    | EventsBody Stream.Reader
    | FileBody File


{-| A file as Bun sends one.
-}
type File
    = File


{-| A plain text response with the status.
-}
text : Int -> String -> Response
text status content =
    Response { status = status, headers = [ ( "Content-Type", "text/plain; charset=utf-8" ) ], body = TextBody content }


{-| A JSON response with the status.
-}
json : Int -> Encode.Value -> Response
json status value =
    Response { status = status, headers = [ ( "Content-Type", "application/json" ) ], body = TextBody (Encode.encode 0 value) }


{-| A response of bytes with the status, of no particular kind
(`application/octet-stream`) until `withHeader` says which.
-}
bytes : Int -> Bytes -> Response
bytes status content =
    Response { status = status, headers = [ ( "Content-Type", "application/octet-stream" ) ], body = BytesBody content }


{-| A response with the status and no body.
-}
empty : Int -> Response
empty status =
    Response { status = status, headers = [], body = NoBody }


{-| A response whose body is what the reader gives, sent as it gives it.
The reader is read only as the client takes what was sent, so a slow
client holds back whatever produces the body. A reader that fails cuts
the response short, and writes a line to stderr.

    Nomi.Http.Server.stream 200 report
        |> Nomi.Http.Server.withHeader "Content-Type" "text/csv"

-}
stream : Int -> Stream.Reader -> Response
stream status reader =
    Response { status = status, headers = [], body = StreamBody reader }


{-| A 200 response of the file, its `Content-Type` by its extension and
its `Content-Length` its size, which Bun sends straight from the file to
the connection. The path is checked against the capability as any is,
when the task runs; the file is opened as it is sent.

    Nomi.Http.Server.file ("public/" ++ String.join "/" (Nomi.Http.Server.path req)) env.fs
        |> Task.onError (\_ -> Task.succeed (Nomi.Http.Server.text 404 "not found"))

-}
file : String -> Fs -> Task Fs.Error Response
file at fs =
    Elm.Kernel.NomiHttpServer.file at fs
        |> Task.map
            (\blob ->
                Response
                    { status = 200
                    , headers = [ ( "Content-Type", contentType (Nomi.Path.extension at) ) ]
                    , body = FileBody blob
                    }
            )


contentType : Maybe String -> String
contentType extension =
    case Maybe.map String.toLower extension of
        Just "html" ->
            "text/html; charset=utf-8"

        Just "css" ->
            "text/css; charset=utf-8"

        Just "js" ->
            "text/javascript; charset=utf-8"

        Just "mjs" ->
            "text/javascript; charset=utf-8"

        Just "json" ->
            "application/json"

        Just "txt" ->
            "text/plain; charset=utf-8"

        Just "csv" ->
            "text/csv; charset=utf-8"

        Just "svg" ->
            "image/svg+xml"

        Just "png" ->
            "image/png"

        Just "jpg" ->
            "image/jpeg"

        Just "jpeg" ->
            "image/jpeg"

        Just "gif" ->
            "image/gif"

        Just "webp" ->
            "image/webp"

        Just "ico" ->
            "image/x-icon"

        Just "wasm" ->
            "application/wasm"

        Just "pdf" ->
            "application/pdf"

        Just "xml" ->
            "application/xml"

        Just "woff" ->
            "font/woff"

        Just "woff2" ->
            "font/woff2"

        Just "map" ->
            "application/json"

        _ ->
            "application/octet-stream"


{-| Adds the header after those the response has, removing any of the
same name, in any case.

    Nomi.Http.Server.text 200 "<p>hi</p>"
        |> Nomi.Http.Server.withHeader "Content-Type" "text/html"

-}
withHeader : String -> String -> Response -> Response
withHeader name value (Response response) =
    let
        other ( existing, _ ) =
            String.toLower existing /= String.toLower name
    in
    Response { response | headers = List.filter other response.headers ++ [ ( name, value ) ] }


{-| Adds the header after those the response has, keeping any of the same
name, for a header sent more than once:

    Nomi.Http.Server.empty 204
        |> Nomi.Http.Server.addHeader "Set-Cookie" "theme=dark"
        |> Nomi.Http.Server.addHeader "Set-Cookie" "lang=en"

-}
addHeader : String -> String -> Response -> Response
addHeader name value (Response response) =
    Response { response | headers = response.headers ++ [ ( name, value ) ] }



-- SERVER-SENT EVENTS


{-| A response of server-sent events, which a browser's `EventSource`
reads: the reader gives them as `event` writes them, each sent as soon as
it is given, and the client reconnects when it ends. The response opens
with a comment, so the client knows the stream is open before the first
event.

    Nomi.Queue.unbounded
        |> Task.andThen (\updates -> Nomi.Stream.reader (Nomi.Queue.take updates |> Task.map (event >> Just)))
        |> Task.map Nomi.Http.Server.events

-}
events : Stream.Reader -> Response
events reader =
    Response
        { status = 200
        , headers = [ ( "Content-Type", "text/event-stream" ), ( "Cache-Control", "no-cache" ) ]
        , body = EventsBody reader
        }


{-| An event as `events` sends it: its data, a line of its own for each of
the data's lines, with its name, which `EventSource` dispatches it by
(`"message"` when there is none), and its id, which a client reconnecting
sends back in `Last-Event-ID`.
-}
event : { name : Maybe String, id : Maybe String, data : String } -> Bytes
event { name, id, data } =
    let
        field key =
            Maybe.map (\value -> key ++ ": " ++ value ++ "\n") >> Maybe.withDefault ""

        lines =
            String.lines data |> List.map (\line -> "data: " ++ line ++ "\n") |> String.concat
    in
    Nomi.Bytes.fromString (field "event" name ++ field "id" id ++ lines ++ "\n")



-- TESTING


{-| A request with the method, target, headers and body, as a client
would send it, for testing a handler. The target is the path and the
query, and its headers are read as a server reads them:

    Nomi.Http.Server.request
        { method = Post
        , target = "/todos?draft=true"
        , headers = [ ( "Content-Type", "application/json" ) ]
        , body = "{\"title\": \"milk\"}"
        }

-}
request : { method : Method, target : String, headers : List ( String, String ), body : String } -> Request
request r =
    Elm.Kernel.NomiHttpServer.request (methodName r.method) r.target r.headers r.body


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
its own, and gives the response written, its body read whole as UTF-8,
each byte that isn't replaced with U+FFFD: a handler that dies answers 500
and logs a line, which a test reads with `Nomi.Test.stderr`, and so does a
response `listen` couldn't write, as status 999 or a line break in a
header. The request's body may hold `defaults`'s `maxBodyBytes`. The
response's headers are as Fetch lists them: names in lower case and in
order, the values of one name joined with `, `, and each `Set-Cookie`
apart. A response with no `Content-Type` and a body gets
`text/plain;charset=utf-8`, as `listen` writes it.

    Nomi.Http.Server.respond handle (Nomi.Http.Server.request { method = Get, target = "/hello/you", headers = [], body = "" })
        |> Task.map (Expect.equal { status = 200, headers = [ ( "content-type", "text/plain; charset=utf-8" ) ], body = "hello, you" })

Interrupting the task is the client disconnecting: the handler is
interrupted and its finalizers run.

-}
respond : (Request -> Task Never Response) -> Request -> Task x { status : Int, headers : List ( String, String ), body : String }
respond handle req =
    Elm.Kernel.NomiHttpServer.answer (unwrap handle) req
