
module Bar exposing (..)

foo x =
    case x of
        False ->
            x
        _ -> not x
            