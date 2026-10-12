
module Foo exposing (..)

import Some exposing (Bar(..))

x =
  let
    (Bar 1) = some
  in
    1
            