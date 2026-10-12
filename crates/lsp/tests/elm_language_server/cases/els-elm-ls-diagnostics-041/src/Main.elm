
module Foo exposing (..)

import Some exposing (Thing, Other)

x : Int -> Other
x y =
  Some.other y
        