module Main exposing (main)

{-| A page about the todo app. It uses no other member, so plain
`elm make src/Main.elm` builds it too.
-}

import Html exposing (Html)
import Html.Attributes as Attr


main : Html msg
main =
    Html.div []
        [ Html.h1 [] [ Html.text "Todos, on Elm at both ends" ]
        , Html.p [] [ Html.text "One list, kept by an Elm server and shown by an Elm page, sharing one definition of a todo." ]
        , Html.a [ Attr.href "http://localhost:8000" ] [ Html.text "Open the app" ]
        ]
