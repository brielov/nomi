port module Main exposing (Model, Shape(..), area, main, notify)

import Html exposing (Html, text)
import Shapes.Circle as Circle


type alias Model =
    { shape : Shape
    , label : String
    }


type Shape
    = Square Float
    | Round Circle.Circle


type alias Hidden =
    Int


port notify : String -> Cmd msg


port incoming : (String -> msg) -> Sub msg


area : Shape -> Float
area shape =
    case shape of
        Square side ->
            side * side

        Round circle ->
            Circle.area circle


describe model =
    let
        twice : Float -> Float
        twice n =
            n * 2

        half n =
            n / 2

        ( first, second ) =
            ( twice (area model.shape), half 1 )
    in
    model.label ++ String.fromFloat (first + second)


hidden : Hidden
hidden =
    0


main : Html msg
main =
    text (describe { shape = Square 2, label = "area " })
