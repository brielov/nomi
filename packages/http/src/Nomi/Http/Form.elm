module Nomi.Http.Form exposing
    ( decode, encode, fields
    , Parts, Part, multipart, fromReader, next
    )

{-| Forms, as browsers send them: URL-encoded fields, and `multipart/form-data`,
whose parts, as files uploaded, are read as streams, one after another.

@docs decode, encode, fields


# Multipart

@docs Parts, Part, multipart, fromReader, next

-}

import Bytes exposing (Bytes)
import Nomi.Bytes
import Nomi.Http.Server as Server exposing (BodyError(..), Request)
import Nomi.Ref as Ref exposing (Ref)
import Nomi.Stream as Stream
import Task exposing (Task)



-- URL-ENCODED


{-| The fields of URL-encoded text, as a form's body or a query: each name
and value percent-decoded, `+` a space. A field with no `=` has an empty
value, and one whose escapes aren't UTF-8 is left out.

    decode "title=milk+%26+eggs&done" -- [ ( "title", "milk & eggs" ), ( "done", "" ) ]

-}
decode : String -> List ( String, String )
decode text =
    String.split "&" text
        |> List.filter (not << String.isEmpty)
        |> List.filterMap
            (\field ->
                case String.indexes "=" field of
                    at :: _ ->
                        Maybe.map2 Tuple.pair (unescape (String.left at field)) (unescape (String.dropLeft (at + 1) field))

                    [] ->
                        Maybe.map (\name -> ( name, "" )) (unescape field)
            )


{-| The fields as URL-encoded text, as `decode` reads them.
-}
encode : List ( String, String ) -> String
encode pairs =
    pairs
        |> List.map (\( name, value ) -> escape name ++ "=" ++ escape value)
        |> String.join "&"


{-| The fields of the request's URL-encoded body.
-}
fields : Request -> Task BodyError (List ( String, String ))
fields req =
    Server.bodyText req |> Task.map decode


unescape : String -> Maybe String
unescape text =
    let
        go chars bytes =
            case chars of
                '%' :: high :: low :: more ->
                    case Nomi.Bytes.fromHex (String.fromList [ high, low ]) of
                        Just byte ->
                            go more (byte :: bytes)

                        Nothing ->
                            go (high :: low :: more) (Nomi.Bytes.fromString "%" :: bytes)

                '+' :: more ->
                    go more (Nomi.Bytes.fromString " " :: bytes)

                c :: more ->
                    go more (Nomi.Bytes.fromString (String.fromChar c) :: bytes)

                [] ->
                    Nomi.Bytes.toString (Nomi.Bytes.concat (List.reverse bytes))
    in
    go (String.toList text) []


escape : String -> String
escape text =
    let
        hex =
            Nomi.Bytes.toHex (Nomi.Bytes.fromString text) |> String.toUpper

        pairs h =
            case String.left 2 h of
                "" ->
                    []

                byte ->
                    byte :: pairs (String.dropLeft 2 h)

        kept byte =
            case Nomi.Bytes.fromHex byte |> Maybe.andThen Nomi.Bytes.toString of
                Just c ->
                    if String.all (\ch -> Char.isAlphaNum ch && Char.toCode ch < 128 || String.contains (String.fromChar ch) "-._~") c then
                        c

                    else if c == " " then
                        "+"

                    else
                        "%" ++ byte

                Nothing ->
                    "%" ++ byte
    in
    String.concat (List.map kept (pairs hex))



-- MULTIPART


{-| The parts of a multipart form, read one after another.
-}
type Parts
    = Parts
        { boundary : Bytes
        , source : Stream.Reader
        , state : Ref State
        }


{-| Where the form's reading is: what was read of it that nothing took
yet, whether its source ended, and what that is: before the first part,
inside a part's body, at the start of the next part's, or past the end.
-}
type alias State =
    { buffer : Bytes
    , ended : Bool
    , at : Place
    }


type Place
    = Preamble
    | InBody
    | Between
    | Done


{-| A part: its field's name, the name of the file it holds, if it is one,
its content type, if it says one, and its body, read as it arrives.
-}
type alias Part =
    { name : String
    , filename : Maybe String
    , contentType : Maybe String
    , body : Stream.Reader
    }


{-| The parts of the request's `multipart/form-data` body, by the boundary
its `Content-Type` gives, or `BadForm` when it gives none.

    upload : Nomi.Fs.Fs -> Request -> Task BodyError ()
    upload fs req =
        Nomi.Http.Form.multipart req
            |> Task.andThen Nomi.Http.Form.next
            |> Task.andThen (\part -> ...)

-}
multipart : Request -> Task BodyError Parts
multipart req =
    case Server.header "Content-Type" req |> Maybe.andThen boundaryOf of
        Just boundary ->
            fromReader boundary (Server.body req)

        Nothing ->
            Task.fail (BadForm "the request is not a multipart form: its Content-Type gives no boundary")


boundaryOf : String -> Maybe String
boundaryOf contentType =
    if String.startsWith "multipart/" (String.toLower (String.trim contentType)) then
        parameters contentType
            |> List.filter (\( key, _ ) -> key == "boundary")
            |> List.head
            |> Maybe.map Tuple.second

    else
        Nothing


{-| The parts of a multipart body the reader gives, with the boundary.
-}
fromReader : String -> Stream.Reader -> Task x Parts
fromReader boundary source =
    Ref.make { buffer = Nomi.Bytes.fromString "", ended = False, at = Preamble }
        |> Task.map (\state -> Parts { boundary = Nomi.Bytes.fromString ("--" ++ boundary), source = source, state = state })


{-| The next part, once the one before was read, or skipped when its body
wasn't read to its end; `Nothing` past the last. A part's body is a reader
of the fiber's that asked for it.
-}
next : Parts -> Task BodyError (Maybe Part)
next parts =
    nextPart parts |> Task.mapError bodyError


nextPart : Parts -> Task Stream.Error (Maybe Part)
nextPart ((Parts p) as parts) =
    Ref.get p.state
        |> Task.andThen
            (\state ->
                case state.at of
                    Done ->
                        Task.succeed Nothing

                    InBody ->
                        skip parts |> Task.andThen (\_ -> nextPart parts)

                    Preamble ->
                        preamble parts |> Task.andThen (\_ -> nextPart parts)

                    Between ->
                        startPart parts
            )


bodyError : Stream.Error -> BodyError
bodyError error =
    case error of
        Stream.TooLarge ->
            TooLarge

        Stream.Invalid why ->
            BadForm why

        _ ->
            Disconnected


{-| Reads more of the source into the buffer, failing once it ended.
-}
fill : Parts -> String -> Task Stream.Error ()
fill (Parts p) what =
    Stream.read p.source
        |> Task.andThen
            (\chunk ->
                case chunk of
                    Just bytes ->
                        Ref.update (\state -> { state | buffer = Nomi.Bytes.append state.buffer bytes }) p.state

                    Nothing ->
                        Task.fail (Stream.Invalid ("the form ends " ++ what))
            )


{-| Takes the buffer up to the first boundary, which starts the parts.
-}
preamble : Parts -> Task Stream.Error ()
preamble ((Parts p) as parts) =
    Ref.get p.state
        |> Task.andThen
            (\state ->
                case Nomi.Bytes.indexOf p.boundary state.buffer of
                    Just at ->
                        Ref.set { state | buffer = Nomi.Bytes.slice (at + Nomi.Bytes.length p.boundary) (Nomi.Bytes.length state.buffer) state.buffer, at = Between } p.state

                    Nothing ->
                        fill parts "before its first part" |> Task.andThen (\_ -> preamble parts)
            )


{-| After a boundary: `--` ends the form, and otherwise a line break and
the part's headers start it.
-}
startPart : Parts -> Task Stream.Error (Maybe Part)
startPart ((Parts p) as parts) =
    Ref.get p.state
        |> Task.andThen
            (\state ->
                if Nomi.Bytes.length state.buffer < 2 then
                    fill parts "after a boundary" |> Task.andThen (\_ -> startPart parts)

                else if Nomi.Bytes.toString (Nomi.Bytes.slice 0 2 state.buffer) == Just "--" then
                    Ref.set { state | at = Done } p.state |> Task.map (\_ -> Nothing)

                else
                    case Nomi.Bytes.indexOf crlfcrlf state.buffer of
                        Just at ->
                            let
                                head =
                                    Nomi.Bytes.slice 0 at state.buffer
                                        |> Nomi.Bytes.toString
                                        |> Maybe.withDefault ""
                            in
                            Ref.set { state | buffer = Nomi.Bytes.slice (at + 4) (Nomi.Bytes.length state.buffer) state.buffer, at = InBody } p.state
                                |> Task.andThen (\_ -> Stream.reader (bodyChunk parts))
                                |> Task.andThen (\body -> part head body)

                        Nothing ->
                            if Nomi.Bytes.length state.buffer > 16384 then
                                Task.fail (Stream.Invalid "a part's headers are longer than 16 KiB")

                            else
                                fill parts "in a part's headers" |> Task.andThen (\_ -> startPart parts)
            )


crlfcrlf : Bytes
crlfcrlf =
    Nomi.Bytes.fromString "\u{000D}\n\u{000D}\n"


{-| The part its headers describe, with its body.
-}
part : String -> Stream.Reader -> Task Stream.Error (Maybe Part)
part head body =
    let
        headers =
            String.split "\u{000D}\n" head
                |> List.filterMap
                    (\line ->
                        case String.indexes ":" line of
                            at :: _ ->
                                Just ( String.toLower (String.trim (String.left at line)), String.trim (String.dropLeft (at + 1) line) )

                            [] ->
                                Nothing
                    )

        headerValue name =
            List.filter (\( key, _ ) -> key == name) headers |> List.head |> Maybe.map Tuple.second

        disposition =
            headerValue "content-disposition" |> Maybe.map parameters |> Maybe.withDefault []

        param name =
            List.filter (\( key, _ ) -> key == name) disposition |> List.head |> Maybe.map Tuple.second
    in
    case param "name" of
        Just name ->
            Task.succeed (Just { name = name, filename = param "filename", contentType = headerValue "content-type", body = body })

        Nothing ->
            Task.fail (Stream.Invalid "a part's Content-Disposition names no field")


{-| A body's next chunk: what comes before the next boundary, holding back
as much as could be the start of one; `Nothing` once the boundary came.
-}
bodyChunk : Parts -> Task Stream.Error (Maybe Bytes)
bodyChunk ((Parts p) as parts) =
    Ref.get p.state
        |> Task.andThen
            (\state ->
                let
                    delimiter =
                        Nomi.Bytes.append (Nomi.Bytes.fromString "\u{000D}\n") p.boundary

                    size =
                        Nomi.Bytes.length state.buffer

                    kept =
                        Nomi.Bytes.length delimiter - 1
                in
                if state.at /= InBody then
                    Task.succeed Nothing

                else
                    case Nomi.Bytes.indexOf delimiter state.buffer of
                        Just at ->
                            Ref.set { state | buffer = Nomi.Bytes.slice (at + Nomi.Bytes.length delimiter) size state.buffer, at = Between } p.state
                                |> Task.map
                                    (\_ ->
                                        if at == 0 then
                                            Nothing

                                        else
                                            Just (Nomi.Bytes.slice 0 at state.buffer)
                                    )

                        Nothing ->
                            if size > kept then
                                Ref.set { state | buffer = Nomi.Bytes.slice (size - kept) size state.buffer } p.state
                                    |> Task.map (\_ -> Just (Nomi.Bytes.slice 0 (size - kept) state.buffer))

                            else
                                fill parts "inside a part" |> Task.andThen (\_ -> bodyChunk parts)
            )


{-| Reads the rest of the current part's body, to drop it.
-}
skip : Parts -> Task Stream.Error ()
skip parts =
    bodyChunk parts
        |> Task.andThen
            (\chunk ->
                case chunk of
                    Just _ ->
                        skip parts

                    Nothing ->
                        Task.succeed ()
            )


{-| The text split at each `;` outside a quoted string.
-}
outsideQuotes : String -> List String
outsideQuotes text =
    let
        step c ( quoted, escaped, ( current, done ) ) =
            if escaped then
                ( quoted, False, ( c :: current, done ) )

            else if quoted && c == '\\' then
                ( quoted, True, ( c :: current, done ) )

            else if c == '"' then
                ( not quoted, False, ( c :: current, done ) )

            else if c == ';' && not quoted then
                ( quoted, False, ( [], String.fromList (List.reverse current) :: done ) )

            else
                ( quoted, False, ( c :: current, done ) )

        ( _, _, ( last, pieces ) ) =
            String.foldl step ( False, False, ( [], [] ) ) text
    in
    List.reverse (String.fromList (List.reverse last) :: pieces)


{-| A header value's parameters, each its lower-case name and its value,
unquoted: `form-data; name="file"; filename="a.txt"`.
-}
parameters : String -> List ( String, String )
parameters value =
    let
        unquoted text =
            if String.startsWith "\"" text && String.endsWith "\"" text && String.length text >= 2 then
                String.slice 1 -1 text |> String.replace "\\\"" "\"" |> String.replace "\\\\" "\\"

            else
                text
    in
    outsideQuotes value
        |> List.drop 1
        |> List.filterMap
            (\param ->
                case String.indexes "=" param of
                    at :: _ ->
                        Just ( String.toLower (String.trim (String.left at param)), unquoted (String.trim (String.dropLeft (at + 1) param)) )

                    [] ->
                        Nothing
            )
