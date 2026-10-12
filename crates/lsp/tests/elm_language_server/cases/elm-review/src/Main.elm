module Main exposing (main)

import Html exposing (Html, text)


unused : Int
unused =
    1


main : Html msg
main =
    text (String.fromInt (1 + 2))
