module Main exposing (main, scale)

import Html exposing (Html, text)


scale : Float -> List Float -> List Float
scale factor values =
    let
        offset =
            factor / 2
    in
    List.map (\value -> value * factor + offset) values


describe : ( Int, String ) -> (Int -> String) -> String
describe ( count, label ) render =
    render count ++ label


main : Html msg
main =
    text (describe ( 2, "items" ) String.fromInt)
