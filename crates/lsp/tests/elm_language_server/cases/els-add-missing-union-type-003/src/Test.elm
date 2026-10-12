module Test exposing (..)

import Html exposing (Html, div)
import Html.Events exposing (onInput)

type Msg
    = Msg1
    | Msg2

view : Html Msg
view =
    div [ onInput NewMsg ] []

