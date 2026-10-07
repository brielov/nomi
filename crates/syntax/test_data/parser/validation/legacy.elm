module Legacy (..) where

import Dict exposing Dict
import Html (div)
import Maybe exposing (Maybe(Just, Nothing))

type alias Point = { x = Int, y : Int }

origin = { x : 0, y = 0 }

pair : (,) Int Int
pair = (,) 1 2

isZero n =
    case n of
        -1 -> False
        0.5 -> False
        -2.5 -> False
        _ -> True

( a, b ) = pair
