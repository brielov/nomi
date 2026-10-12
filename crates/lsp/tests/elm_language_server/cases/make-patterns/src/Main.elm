module Main exposing (main)

import Html exposing (Html, text)


type Color
    = Red
    | Green
    | Blue


name : Color -> String
name color =
    case color of
        Red ->
            "red"

        Green ->
            "green"


main : Html msg
main =
    text (name Blue)
