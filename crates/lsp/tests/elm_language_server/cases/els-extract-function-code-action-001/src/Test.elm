module Test exposing (..)

val2 =
    0

foo val str =
    case val of
        Just { field1, field2, field3 } ->
            case field1 of
                Just { prop1, prop2 } ->
                    prop1 + prop2 + val + field2

                Nothing ->
                    field2 + val2

        Nothing ->
            str + val2

