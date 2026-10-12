module Main exposing (..)

import B exposing (C(..))

f : Int -> C
f x =
    if x > 1 then
        Something
    else
        SomethingElse

