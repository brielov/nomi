module Shapes exposing (Shape(..), sides)

{-| Shapes.

@docs Shape, sides

-}

import Internal


{-| A shape.
-}
type Shape
    = Square
    | Triangle


{-| How many sides a shape has.
-}
sides : Shape -> Int
sides shape =
    Internal.count shape
