
module Foo exposing (foo)

type alias Person = { name : String, age : Int}

port foo : Person -> Cmd msg
            