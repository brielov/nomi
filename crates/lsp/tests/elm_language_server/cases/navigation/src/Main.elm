module Main exposing (main, view)

import Html exposing (Html, text)
import Shapes exposing (Shape(..), area)


main : Html msg
main =
    view (Circle 2)
  --^   --^


view : Shape -> Html msg
     --^
view shape =
    text (Shapes.describe shape ++ String.fromFloat (area shape))
                --^                    --^          --^
