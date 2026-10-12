
module Foo exposing (foo)

import Some exposing (Bar(..))

x =
  let
    (Bar 1) = some
  in
    1
            