module FooExtra exposing (..)

import Module as M

func : MyType -> String
func a =
    case a of
        M.B ->
          ""

        M.A ->
      --X
          ""

