module Main exposing (main, view)

import Html exposing (Html, text)


main : Html msg

--^

view : Html msg
view =
    text "view"


--^
helper =
    1


update model =
    case model of
        name ->
            name
          --^
