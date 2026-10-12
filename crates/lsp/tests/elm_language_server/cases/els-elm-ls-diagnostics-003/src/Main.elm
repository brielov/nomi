
module Bar exposing (..)

foo x =
    case x of
        Some ->
            x
        Other -> not x
            