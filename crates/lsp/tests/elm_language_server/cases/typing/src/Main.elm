module Main exposing (main)

import Html exposing (Html, text)


main : Html msg
main =
    text (String.fromInt (List.length numbers))


numbers : List Int
numbers =
    [ 1, 2, 3 ]
