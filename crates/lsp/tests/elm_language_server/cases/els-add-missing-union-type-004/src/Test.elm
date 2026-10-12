module Test exposing (..)

import Html exposing (Html, div)
import Html.Events exposing (onClick)

type Msg
    = Msg1

view : Html Msg
view =
    div [ onClick NewMsg ] []

