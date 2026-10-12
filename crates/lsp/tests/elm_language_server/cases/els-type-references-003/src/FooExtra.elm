module FooExtra exposing (..)

import Module exposing (MyType(..))

func : MyType -> String
func a =
    case a of
        B ->
          ""

        A ->
      --X
          ""

