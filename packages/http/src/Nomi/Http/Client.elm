module Nomi.Http.Client exposing
    ( Request, get, post, put, patch, delete, request
    , withHeader, withBytes, withText, withJson, withTimeout, withLimit
    , send, sendBytes, sendText, sendJson, Response, header
    , Error(..), errorToString
    )

{-| An HTTP client, on `fetch`. Building a request is pure; sending it
takes the network, so a program reaches only the hosts its `Net` allows.
A status is data, not a failure: a 404 is a response.

    Nomi.Http.Client.get "https://api.github.com/zen"
        |> Nomi.Http.Client.withHeader "User-Agent" "nomi"
        |> (\req -> Nomi.Http.Client.sendText req env.net)


# Requests

@docs Request, get, post, put, patch, delete, request
@docs withHeader, withBytes, withText, withJson, withTimeout, withLimit


# Sending

@docs send, sendBytes, sendText, sendJson, Response, header


# Errors

@docs Error, errorToString

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiHttpClient
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Nomi.Bytes
import Nomi.Net exposing (Net)
import Nomi.Stream as Stream
import Task exposing (Task)



-- REQUESTS


{-| A request to send.
-}
type Request
    = Request
        { method : String
        , url : String
        , headers : List ( String, String )
        , body : Maybe Bytes
        , timeout : Maybe Int
        , limit : Int
        }


{-| A request with the method, as `"GET"` or `"PROPFIND"`, to the URL.
-}
request : String -> String -> Request
request method url =
    Request { method = method, url = url, headers = [], body = Nothing, timeout = Nothing, limit = 16 * 1024 * 1024 }


{-| A GET request to the URL.
-}
get : String -> Request
get =
    request "GET"


{-| A POST request to the URL.
-}
post : String -> Request
post =
    request "POST"


{-| A PUT request to the URL.
-}
put : String -> Request
put =
    request "PUT"


{-| A PATCH request to the URL.
-}
patch : String -> Request
patch =
    request "PATCH"


{-| A DELETE request to the URL.
-}
delete : String -> Request
delete =
    request "DELETE"


{-| Adds the header to those the request sends.
-}
withHeader : String -> String -> Request -> Request
withHeader name value (Request r) =
    Request { r | headers = r.headers ++ [ ( name, value ) ] }


{-| Sends the bytes as the body, with the content type, in place of
another body.
-}
withBytes : String -> Bytes -> Request -> Request
withBytes contentType content (Request r) =
    Request { r | body = Just content, headers = withoutType r.headers ++ [ ( "Content-Type", contentType ) ] }


{-| Sends the text as the body, in UTF-8.
-}
withText : String -> Request -> Request
withText content =
    withBytes "text/plain; charset=utf-8" (Nomi.Bytes.fromString content)


{-| Sends the JSON as the body.
-}
withJson : Encode.Value -> Request -> Request
withJson value =
    withBytes "application/json" (Nomi.Bytes.fromString (Encode.encode 0 value))


withoutType : List ( String, String ) -> List ( String, String )
withoutType =
    List.filter (\( name, _ ) -> String.toLower name /= "content-type")


{-| Gives up, with `Timeout`, once this many milliseconds passed before the
response came, redirects included. A body streamed after it came is its
reader's to time.
-}
withTimeout : Int -> Request -> Request
withTimeout ms (Request r) =
    Request { r | timeout = Just ms }


{-| The most bytes `sendBytes`, `sendText` and `sendJson` read of the
response's body, failing with `BadBody` past it: 16 MiB unless given.
-}
withLimit : Int -> Request -> Request
withLimit bytes (Request r) =
    Request { r | limit = bytes }



-- SENDING


{-| What came back: the status, the headers as Fetch lists them (names in
lower case and in order, the values of one name joined with `, `, each
`Set-Cookie` apart), and the body.
-}
type alias Response body =
    { status : Int
    , headers : List ( String, String )
    , body : body
    }


{-| The value of the response's header with the name, in any case.
-}
header : String -> Response body -> Maybe String
header name response =
    List.filter (\( key, _ ) -> key == String.toLower name) response.headers
        |> List.head
        |> Maybe.map Tuple.second


{-| Sends the request, following redirects, and gives the response once its
headers came, its body a reader of what follows. Each redirect is a
request to a host the network must allow too, and one to another host
sends no `Authorization` or `Cookie` header. Interrupting the task aborts
the request, and the body is the fiber's that sent it: once that fiber
ends, what is left unread is dropped.
-}
send : Request -> Net -> Task Error (Response Stream.Reader)
send (Request r) =
    Elm.Kernel.NomiHttpClient.send r


{-| Sends the request and reads its body whole.
-}
sendBytes : Request -> Net -> Task Error (Response Bytes)
sendBytes ((Request r) as req) net =
    send req net
        |> Task.andThen
            (\response ->
                Stream.readAll { limit = r.limit } response.body
                    |> Task.mapError (Stream.errorToString >> BadBody)
                    |> Task.map (\all -> { status = response.status, headers = response.headers, body = all })
            )


{-| Sends the request and reads its body as UTF-8 text, failing with
`BadBody` when it isn't.
-}
sendText : Request -> Net -> Task Error (Response String)
sendText req net =
    sendBytes req net
        |> Task.andThen
            (\response ->
                case Nomi.Bytes.toString response.body of
                    Just content ->
                        Task.succeed { status = response.status, headers = response.headers, body = content }

                    Nothing ->
                        Task.fail (BadBody "the body is not UTF-8")
            )


{-| Sends the request and decodes its body as JSON, whatever its status,
failing with `BadBody` when it isn't the JSON expected.

    Nomi.Http.Client.sendJson (Decode.field "login" Decode.string) req env.net

-}
sendJson : Decoder a -> Request -> Net -> Task Error (Response a)
sendJson decoder req net =
    sendText req net
        |> Task.andThen
            (\response ->
                case Decode.decodeString decoder response.body of
                    Ok value ->
                        Task.succeed { status = response.status, headers = response.headers, body = value }

                    Err error ->
                        Task.fail (BadBody (Decode.errorToString error))
            )



-- ERRORS


{-| Why no response came: the URL isn't one, the network doesn't allow
its host, the host couldn't be reached or answered with no response, the
timeout passed, the redirects went on past ten, or the body couldn't be
read as asked.
-}
type Error
    = BadUrl String
    | NotAllowed String
    | Unreachable String
    | Timeout
    | TooManyRedirects
    | BadBody String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        BadUrl url ->
            url ++ ": not a URL"

        NotAllowed host ->
            host ++ ": not a host the network allows"

        Unreachable why ->
            why

        Timeout ->
            "the request timed out"

        TooManyRedirects ->
            "the request was redirected more than ten times"

        BadBody why ->
            "the response's body: " ++ why
