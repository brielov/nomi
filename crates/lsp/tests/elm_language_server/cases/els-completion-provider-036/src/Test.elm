module Test exposing (..)


type State
    = State { field1 : String, field2 : String }


testFunction : State -> a
testFunction (State state) =
    [ state. ]

