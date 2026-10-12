module Indirect exposing (..)

import Bar exposing (Bar)

foo : Bar -> String
foo arg  =
    arg.foo.field
           --X

update : Bar -> Bar
update arg =
    let
      foo = arg.foo
    in
    { arg | foo = { foo | field = "" } }
                          --X

test : Bar
test =
    { foo = { field = "" } }
              --X

destructure : Bar -> String
destructure { foo } =
    let
      { field } = foo
        --X

    in
    field
    --X

