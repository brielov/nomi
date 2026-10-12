module Test exposing (..)

import Html

type Model = Html
            --^

value : Model
value = Html
       --X

view : Html.Html msg
view = Html.p [] []

