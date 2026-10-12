module Test exposing (..)

val2 =
    0

foo val str =
    case val of
        Just { field1, field2, field3 } ->
            (\{ prop1, prop2 } ->
                prop1 + prop2 + val + field2)
            field1

        Nothing ->
            str + val2

