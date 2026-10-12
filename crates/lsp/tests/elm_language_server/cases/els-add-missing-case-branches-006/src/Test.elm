module Test exposing (..)

type Foo
    = Bar
    | Biz

test : Foo -> ()
test response =
    case response of
        Bar ->
            ()


