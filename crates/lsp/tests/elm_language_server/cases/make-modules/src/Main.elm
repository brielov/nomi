module Main exposing (main)

import Html exposing (Html, text)
import Shapes


main : Html msg
main =
    text (String.fromInt (Shapes.sides Shapes.Square))
