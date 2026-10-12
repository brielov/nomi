module Test exposing (..)

val2 =
    0

type alias Field1 =
    { prop1: Int, prop2: Int }

foo : Maybe { field1: Field1, field2: Int, field3: Int } -> Int -> Int
foo val str =
    case val of
        Just { field1, field2, field3 } ->
            (\{ prop1, prop2 } ->
                prop1 + prop2 + val + field2)
            field1

        Nothing ->
            str + val2

