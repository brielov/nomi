module Nomi.Http.Cookie exposing
    ( parse, get
    , SetCookie, SameSite(..), cookie, removal, serialize
    , sign, verify
    )

{-| Cookies, as RFC 6265 has them: those a request sends, in its `Cookie`
header, and those a response sets, each in a `Set-Cookie` header.

    case Nomi.Http.Server.header "Cookie" req |> Maybe.andThen (Nomi.Http.Cookie.get "theme") of
        Just theme ->
            ...

    Nomi.Http.Server.text 200 "ok"
        |> Nomi.Http.Server.addHeader "Set-Cookie" (Nomi.Http.Cookie.serialize (Nomi.Http.Cookie.cookie "theme" "dark"))

A cookie's value is a client's to change, so one the server must trust,
as a session's id, is signed with a secret only the server knows, and
verified when it comes back.

@docs parse, get


# Setting

@docs SetCookie, SameSite, cookie, removal, serialize


# Signing

@docs sign, verify

-}

import Bytes exposing (Bytes)
import Nomi.Bytes
import Nomi.Crypto as Crypto
import Time



-- READING


{-| The cookies a `Cookie` header holds, each its name and value, in order:
`parse "SID=31d4d96e407aad42; lang=en-US"` is
`[ ( "SID", "31d4d96e407aad42" ), ( "lang", "en-US" ) ]`. A pair with no
`=` or no name is left out.
-}
parse : String -> List ( String, String )
parse header =
    String.split ";" header
        |> List.filterMap
            (\pair ->
                case String.indexes "=" pair of
                    at :: _ ->
                        let
                            name =
                                String.trim (String.left at pair)
                        in
                        if String.isEmpty name then
                            Nothing

                        else
                            Just ( name, String.trim (String.dropLeft (at + 1) pair) )

                    [] ->
                        Nothing
            )


{-| The value of the first cookie of the name a `Cookie` header holds.
-}
get : String -> String -> Maybe String
get name header =
    parse header
        |> List.filter (\( key, _ ) -> key == name)
        |> List.head
        |> Maybe.map Tuple.second



-- SETTING


{-| A cookie for a response to set: its name and value, when it expires,
by a time or after so many seconds (neither: when the browser closes),
the domain and path it is sent to, and whether it is sent only over
HTTPS (`secure`), kept from the page's scripts (`httpOnly`), and sent
with requests other sites start (`sameSite`).

A value is sent as it is, so it should hold no space, quote, comma,
semicolon or backslash: `Nomi.Bytes.toBase64Url` turns any bytes into
such a value.

-}
type alias SetCookie =
    { name : String
    , value : String
    , expires : Maybe Time.Posix
    , maxAge : Maybe Int
    , domain : Maybe String
    , path : Maybe String
    , secure : Bool
    , httpOnly : Bool
    , sameSite : Maybe SameSite
    }


{-| When a browser sends the cookie with a request another site starts:
never, only when following a link to the site, or always, which needs
`secure`.
-}
type SameSite
    = Strict
    | Lax
    | None


{-| A cookie of the name and value for the whole site, until the browser
closes, kept from scripts, and sent with other sites' links but not their
requests:

    { name = name, value = value, expires = Nothing, maxAge = Nothing, domain = Nothing
    , path = Just "/", secure = False, httpOnly = True, sameSite = Just Lax }

-}
cookie : String -> String -> SetCookie
cookie name value =
    { name = name
    , value = value
    , expires = Nothing
    , maxAge = Nothing
    , domain = Nothing
    , path = Just "/"
    , secure = False
    , httpOnly = True
    , sameSite = Just Lax
    }


{-| A cookie that removes the one of the name, for the whole site.
-}
removal : String -> SetCookie
removal name =
    let
        gone =
            cookie name ""
    in
    { gone | expires = Just (Time.millisToPosix 0), maxAge = Just 0 }


{-| The cookie as a `Set-Cookie` header's value:
`"SID=31d4d96e407aad42; Path=/; Secure; HttpOnly"`.
-}
serialize : SetCookie -> String
serialize c =
    [ Just (c.name ++ "=" ++ c.value)
    , Maybe.map (\time -> "Expires=" ++ httpDate time) c.expires
    , Maybe.map (\seconds -> "Max-Age=" ++ String.fromInt seconds) c.maxAge
    , Maybe.map (\path -> "Path=" ++ path) c.path
    , Maybe.map (\domain -> "Domain=" ++ domain) c.domain
    , if c.secure then
        Just "Secure"

      else
        Nothing
    , if c.httpOnly then
        Just "HttpOnly"

      else
        Nothing
    , Maybe.map (\sameSite -> "SameSite=" ++ sameSiteName sameSite) c.sameSite
    ]
        |> List.filterMap identity
        |> String.join "; "


sameSiteName : SameSite -> String
sameSiteName sameSite =
    case sameSite of
        Strict ->
            "Strict"

        Lax ->
            "Lax"

        None ->
            "None"


{-| The time as HTTP writes it: `"Wed, 09 Jun 2021 10:18:14 GMT"`.
-}
httpDate : Time.Posix -> String
httpDate time =
    let
        two n =
            String.padLeft 2 '0' (String.fromInt n)

        weekday =
            case Time.toWeekday Time.utc time of
                Time.Mon ->
                    "Mon"

                Time.Tue ->
                    "Tue"

                Time.Wed ->
                    "Wed"

                Time.Thu ->
                    "Thu"

                Time.Fri ->
                    "Fri"

                Time.Sat ->
                    "Sat"

                Time.Sun ->
                    "Sun"

        month =
            case Time.toMonth Time.utc time of
                Time.Jan ->
                    "Jan"

                Time.Feb ->
                    "Feb"

                Time.Mar ->
                    "Mar"

                Time.Apr ->
                    "Apr"

                Time.May ->
                    "May"

                Time.Jun ->
                    "Jun"

                Time.Jul ->
                    "Jul"

                Time.Aug ->
                    "Aug"

                Time.Sep ->
                    "Sep"

                Time.Oct ->
                    "Oct"

                Time.Nov ->
                    "Nov"

                Time.Dec ->
                    "Dec"
    in
    weekday
        ++ ", "
        ++ two (Time.toDay Time.utc time)
        ++ " "
        ++ month
        ++ " "
        ++ String.fromInt (Time.toYear Time.utc time)
        ++ " "
        ++ two (Time.toHour Time.utc time)
        ++ ":"
        ++ two (Time.toMinute Time.utc time)
        ++ ":"
        ++ two (Time.toSecond Time.utc time)
        ++ " GMT"



-- SIGNING


{-| The value, signed with the secret for a cookie of the name: the value,
a `.`, and the HMAC-SHA-256 of the name and value in base64 for URLs. The
name is signed too, so a value signed for one cookie isn't taken for
another's.

    Nomi.Http.Cookie.cookie "session" (Nomi.Http.Cookie.sign secret "session" id)

-}
sign : Bytes -> String -> String -> String
sign secret name value =
    value ++ "." ++ signature secret name value


{-| The value a signed value holds, when its signature is the secret's for
a cookie of the name, and `Nothing` for one a client made or changed.
-}
verify : Bytes -> String -> String -> Maybe String
verify secret name signed =
    case List.reverse (String.indexes "." signed) of
        at :: _ ->
            let
                value =
                    String.left at signed

                given =
                    Nomi.Bytes.fromString (String.dropLeft (at + 1) signed)
            in
            if Crypto.equalConstantTime given (Nomi.Bytes.fromString (signature secret name value)) then
                Just value

            else
                Nothing

        [] ->
            Nothing


signature : Bytes -> String -> String -> String
signature secret name value =
    Crypto.hmac Crypto.Sha256 secret (Nomi.Bytes.fromString (name ++ "=" ++ value))
        |> Nomi.Bytes.toBase64Url
