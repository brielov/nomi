module Main exposing (main)

import Html exposing (Html, text)


greeting =
    "hello"


add a b =
    a + b


pair =
    ( 1, "one" )


main : Html msg
main =
    text (greeting ++ String.fromInt (add 1 2) ++ Tuple.second pair)
