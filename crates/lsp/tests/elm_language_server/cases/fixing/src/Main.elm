module Main exposing (main)

import Html exposing (Html, text)
import Shapes.Circle as Circle


main : Html msg
main =
    text (String.fromFloat (Circle.area (Circle.circle 2)))
