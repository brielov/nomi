module Module exposing (..)

type alias Foo a b =
             --^
    { bar : a
          --X
    , baz : b
    }

