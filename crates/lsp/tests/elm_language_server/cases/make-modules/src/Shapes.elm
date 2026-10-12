module Shapes exposing (Shape(..), sides)


type Shape
    = Square
    | Triangle


sides : Shape -> Int
sides shape =
    case shape of
        Square ->
            "four"

        Triangle ->
            3
