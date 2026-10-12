module A exposing (..)

import B
import B.Nested


value : B.Value
value =
    B.Value


unwrap input =
    case input of
        B.Value ->
            B.value


nested =
    B.Nested.value
