module Test exposing (..)

type alias Foo =
    { foo : Bar
    }


type alias Bar =
    { bar : Int }
    --^


fooBarList : List Foo
fooBarList =
    [ { foo = { bar = 0 } }
    , { foo = { bar = 1 } }
    ]

