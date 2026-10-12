module Test exposing (..)

import Html exposing (Html, div)
import Html.Events exposing (onClick)

import Module exposing (Msg(..))

view : Html Msg
view =
    div [ onClick NewMsg ] []

