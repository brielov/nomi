module Test exposing (..)

import Foo

test : Foo -> Html.Html msg
test response =
    case response of
        Foo.Bar ->
            Html.text "x"

