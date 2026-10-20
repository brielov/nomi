module WebTest exposing (suite)

import Bytes exposing (Bytes)
import Expect
import Fuzz
import Nomi.Bytes
import Nomi.Http.Cookie as Cookie
import Nomi.Http.Form as Form
import Nomi.Http.Route as Route exposing (Route)
import Nomi.Http.Server as Server exposing (Method(..))
import Nomi.Ref as Ref
import Nomi.Stream as Stream
import Nomi.Test exposing (effect)
import Task exposing (Task)
import Test exposing (Test, describe, fuzz, test)
import Time


suite : Test
suite =
    describe "nomi/http's routes, cookies and forms"
        [ routes
        , cookies
        , forms
        ]



-- ROUTES


type Page
    = Home
    | Todo Int
    | EditTodo Int
    | Search String
    | Asset (List String)


page : Route (Page -> a) a
page =
    Route.oneOf
        [ Route.map Home Route.top
        , Route.map Todo (Route.s "todos" |> Route.slash Route.int)
        , Route.map EditTodo (Route.s "todos" |> Route.slash Route.int |> Route.slash (Route.s "edit"))
        , Route.map Search (Route.s "search" |> Route.slash Route.string)
        , Route.map Asset (Route.s "assets" |> Route.slash Route.rest)
        ]


routes : Test
routes =
    test "a table of paths" <|
        \_ ->
            [ ( [], Just Home )
            , ( [ "todos", "3" ], Just (Todo 3) )
            , ( [ "todos", "-3" ], Just (Todo -3) )
            , ( [ "todos", "three" ], Nothing )
            , ( [ "todos", "3", "edit" ], Just (EditTodo 3) )
            , ( [ "todos", "3", "delete" ], Nothing )
            , ( [ "todos" ], Nothing )
            , ( [ "search", "milk & eggs" ], Just (Search "milk & eggs") )
            , ( [ "assets", "css", "site.css" ], Just (Asset [ "css", "site.css" ]) )
            , ( [ "assets" ], Just (Asset []) )
            , ( [ "elsewhere" ], Nothing )
            ]
                |> List.map (\( path, _ ) -> Route.parse page path)
                |> Expect.equal
                    [ Just Home
                    , Just (Todo 3)
                    , Just (Todo -3)
                    , Nothing
                    , Just (EditTodo 3)
                    , Nothing
                    , Nothing
                    , Just (Search "milk & eggs")
                    , Just (Asset [ "css", "site.css" ])
                    , Just (Asset [])
                    , Nothing
                    ]



-- COOKIES


secret : Bytes
secret =
    Nomi.Bytes.fromString "a secret only the server knows"


cookies : Test
cookies =
    describe "cookies"
        [ test "a Cookie header, as RFC 6265 sends one" <|
            \_ ->
                ( Cookie.parse "SID=31d4d96e407aad42; lang=en-US", Cookie.get "lang" "SID=31d4d96e407aad42; lang=en-US" )
                    |> Expect.equal ( [ ( "SID", "31d4d96e407aad42" ), ( "lang", "en-US" ) ], Just "en-US" )
        , test "a header with stray pairs" <|
            \_ ->
                Cookie.parse " a=1 ;; =2; b; c==3;d="
                    |> Expect.equal [ ( "a", "1" ), ( "c", "=3" ), ( "d", "" ) ]
        , test "RFC 6265's Set-Cookie headers" <|
            \_ ->
                let
                    plain name value =
                        Cookie.cookie name value |> (\c -> { c | path = Nothing, httpOnly = False, sameSite = Nothing })
                in
                [ plain "SID" "31d4d96e407aad42" |> (\c -> { c | path = Just "/", domain = Just "example.com" })
                , plain "SID" "31d4d96e407aad42" |> (\c -> { c | path = Just "/", secure = True, httpOnly = True })
                , plain "lang" "en-US" |> (\c -> { c | path = Just "/", domain = Just "example.com" })
                , plain "lang" "en-US" |> (\c -> { c | expires = Just (Time.millisToPosix 1623233894000) })
                , plain "lang" "" |> (\c -> { c | expires = Just (Time.millisToPosix 784111777000) })
                ]
                    |> List.map Cookie.serialize
                    |> Expect.equal
                        [ "SID=31d4d96e407aad42; Path=/; Domain=example.com"
                        , "SID=31d4d96e407aad42; Path=/; Secure; HttpOnly"
                        , "lang=en-US; Path=/; Domain=example.com"
                        , "lang=en-US; Expires=Wed, 09 Jun 2021 10:18:14 GMT"
                        , "lang=; Expires=Sun, 06 Nov 1994 08:49:37 GMT"
                        ]
        , test "the defaults, and a removal" <|
            \_ ->
                [ Cookie.cookie "theme" "dark", Cookie.removal "theme" ]
                    |> List.map Cookie.serialize
                    |> Expect.equal
                        [ "theme=dark; Path=/; HttpOnly; SameSite=Lax"
                        , "theme=; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/; HttpOnly; SameSite=Lax"
                        ]
        , fuzz Fuzz.string "a signed value verifies" <|
            \value -> Cookie.verify secret "session" (Cookie.sign secret "session" value) |> Expect.equal (Just value)
        , test "a tampered one doesn't" <|
            \_ ->
                let
                    signed =
                        Cookie.sign secret "session" "user-7"
                in
                [ Cookie.verify secret "session" (String.replace "user-7" "user-1" signed)
                , Cookie.verify secret "session" (signed ++ "x")
                , Cookie.verify secret "other" signed
                , Cookie.verify (Nomi.Bytes.fromString "another secret") "session" signed
                , Cookie.verify secret "session" "user-7"
                ]
                    |> Expect.equal [ Nothing, Nothing, Nothing, Nothing, Nothing ]
        ]



-- FORMS


multipartBody : String
multipartBody =
    String.join "\u{000D}\n"
        [ "a preamble, which is skipped"
        , "--XyZ"
        , "Content-Disposition: form-data; name=\"title\""
        , ""
        , "milk"
        , "--XyZ"
        , "Content-Disposition: form-data; name=\"file\"; filename=\"notes;v2.txt\""
        , "Content-Type: text/plain"
        , ""
        , "line one\u{000D}\nline two, with --XyZ not at a line's start"
        , "--XyZ"
        , "Content-Disposition: form-data; name=\"skipped\""
        , ""
        , "never read"
        , "--XyZ"
        , "Content-Disposition: form-data; name=\"empty\""
        , ""
        , ""
        , "--XyZ--"
        , "an epilogue"
        ]


forms : Test
forms =
    describe "forms"
        [ test "URL-encoded fields" <|
            \_ ->
                Form.decode "title=milk+%26+eggs&done&caf%C3%A9=%E2%9C%93&bad=%FF&&x=%2"
                    |> Expect.equal [ ( "title", "milk & eggs" ), ( "done", "" ), ( "café", "✓" ), ( "x", "%2" ) ]
        , test "encoded as forms are" <|
            \_ ->
                Form.encode [ ( "title", "milk & eggs" ), ( "café", "✓ 100%" ) ]
                    |> Expect.equal "title=milk+%26+eggs&caf%C3%A9=%E2%9C%93+100%25"
        , fuzz (Fuzz.list (Fuzz.pair Fuzz.string Fuzz.string)) "fields round-trip" <|
            \pairs ->
                Form.decode (Form.encode pairs) |> Expect.equal pairs
        , effect "a multipart form, its parts read in chunks of 1, 7 and 1000 bytes" <|
            \_ ->
                [ 1, 7, 1000 ]
                    |> List.map (\size -> chunked size multipartBody |> Task.andThen (Form.fromReader "XyZ") |> Task.andThen readParts)
                    |> Task.sequence
                    |> Task.mapError Debug.toString
                    |> Task.map
                        (Expect.equal
                            (List.repeat 3
                                [ "title=milk"
                                , "file (notes;v2.txt, text/plain)=line one\u{000D}\nline two, with --XyZ not at a line's start"
                                , "skipped (skipped)"
                                , "empty="
                                ]
                            )
                        )
        , effect "a request's multipart body, by its Content-Type's boundary" <|
            \_ ->
                Server.request { method = Post, target = "/", headers = [ ( "Content-Type", "multipart/form-data; boundary=\"XyZ\"" ) ], body = multipartBody }
                    |> Form.multipart
                    |> Task.andThen readParts
                    |> Task.map List.length
                    |> Task.mapError Debug.toString
                    |> Task.map (Expect.equal 4)
        , effect "a form cut short, or not a form, fails with BadForm" <|
            \_ ->
                [ chunked 1000 (String.left 120 multipartBody) |> Task.andThen (Form.fromReader "XyZ") |> Task.andThen readParts |> Task.map (\_ -> "read")
                , Server.request { method = Post, target = "/", headers = [ ( "Content-Type", "text/plain" ) ], body = "" } |> Form.multipart |> Task.map (\_ -> "read")
                ]
                    |> List.map (Task.onError (\error -> Task.succeed (Debug.toString error |> String.left 7)))
                    |> Task.sequence
                    |> Task.map (Expect.equal [ "BadForm", "BadForm" ])
        ]


{-| Each part's name and body, but `skipped`'s, whose body is left unread.
-}
readParts : Form.Parts -> Task Server.BodyError (List String)
readParts parts =
    Form.next parts
        |> Task.andThen
            (\next ->
                case next of
                    Nothing ->
                        Task.succeed []

                    Just part ->
                        (if part.name == "skipped" then
                            Task.succeed "skipped (skipped)"

                         else
                            Stream.readAll { limit = 10000 } part.body
                                |> Task.mapError (\_ -> Server.Disconnected)
                                |> Task.map
                                    (\body ->
                                        part.name
                                            ++ (case part.filename of
                                                    Just filename ->
                                                        " (" ++ filename ++ ", " ++ Maybe.withDefault "?" part.contentType ++ ")"

                                                    Nothing ->
                                                        ""
                                               )
                                            ++ "="
                                            ++ Maybe.withDefault "?" (Nomi.Bytes.toString body)
                                    )
                        )
                            |> Task.andThen (\line -> readParts parts |> Task.map ((::) line))
            )


{-| A reader of the text in chunks of `size` bytes.
-}
chunked : Int -> String -> Task x Stream.Reader
chunked size text =
    Ref.make (Nomi.Bytes.fromString text)
        |> Task.andThen
            (\left ->
                Stream.reader
                    (Ref.modify
                        (\bytes ->
                            ( if Nomi.Bytes.length bytes == 0 then
                                Nothing

                              else
                                Just (Nomi.Bytes.slice 0 size bytes)
                            , Nomi.Bytes.slice size (Nomi.Bytes.length bytes) bytes
                            )
                        )
                        left
                    )
            )
