module Main exposing (helper, main)

import Html exposing (Html, text)
import Utils


{-| Doubles a number.
-}
helper : Int -> Int
helper n =
    n * 2


main : Html msg
main =
    text (String.fromInt (helper (Utils.base + 1)))
