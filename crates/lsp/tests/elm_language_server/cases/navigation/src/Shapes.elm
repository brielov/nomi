module Shapes exposing (Shape(..), Size, area, describe)

{-| Shapes and their areas.
-}


{-| A shape.
-}
type Shape
    = Circle Float
    | Rectangle Size


{-| Width and height.
-}
type alias Size =
    { width : Float, height : Float }


{-| The area of a shape.
-}
area : Shape -> Float
area shape =
    case shape of
        Circle radius ->
            pi * radius * radius

        Rectangle size ->
            size.width * size.height
                       --^


describe : Shape -> String
describe shape =
    let
        label =
            case shape of
                Circle _ ->
                    "circle"

                Rectangle _ ->
                    "rectangle"
    in
    label ++ " of area " ++ String.fromFloat (area shape)
  --^
