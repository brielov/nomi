
module Bar exposing (..)

foo x =
    case x of
        True ->
            x
        _ -> not x
			