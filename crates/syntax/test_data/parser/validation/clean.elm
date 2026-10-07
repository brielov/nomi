module Clean exposing (main)

{-| Docs. -}

import Html exposing (Html, text)


main : Html msg
main =
    case ( 1, "x" ) of
        ( 1, _ ) -> text "one"
        _ -> text { name = "x" }.name
