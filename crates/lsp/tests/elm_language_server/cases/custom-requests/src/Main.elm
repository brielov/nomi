module Main exposing (main, view)

{-| The structure of a module.
-}

import Html exposing (Html, text)
import Html.Attributes as Attributes
-- a comment between imports
import Shapes.Circle exposing (Circle)


{-| A color.
-}
type Color
    = Red
    | Green -- the second
    | Blue


-- COLORS


type alias Point =
    { x : Float
    , y : Float
    }


-- region helpers


describe : Color -> String
describe color =
    case color of
        Red ->
            "red"

        Green ->
            if True then
                "green"
            else
                "not green"

        Blue ->
            let
                name =
                    "blue"

                other =
                    name
            in
            other


-- endregion


origin : Point
origin =
    { x = 0, y = 0 }
       --^


main : Html msg
--^
main =
    view (Circle 1)


view : Circle -> Html msg
view circle =
    text (describe Red ++ String.fromFloat circle.radius)
                                                --^
