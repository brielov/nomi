module Test exposing (..)

import Foo

test : Foo -> ()
test response =
    case response of
        Foo.Bar ->
            ()


