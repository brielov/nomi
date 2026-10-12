module Bar exposing (Bar)

import Module exposing (Foo)

type alias Bar = { foo : Foo }

foo : Foo -> String
foo arg  =
    arg.field
        --X

update : Foo -> Foo
update arg =
    { arg | field = "" }
            --X

test : Foo
test =
    { field = "" }
      --X

destructure : Foo -> String
destructure { field } =
              --X
    field
    --X

