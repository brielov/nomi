module Main exposing (main)

import Html
import Model


main : Html.Html msg
main =
    view example


example : Model.IAmUsed
example =
    { used = True
    , optional = False
    }


view : Model.IAmUsed -> Html.Html msg
view blah =
    Html.text <|
        if blah.used then
            "used"

        else
            "not used"
            
