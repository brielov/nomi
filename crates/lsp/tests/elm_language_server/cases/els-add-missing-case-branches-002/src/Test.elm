module Test exposing (..)

type Maybe a = Just a | Nothing

func a b =
    case a of
        1 ->
            ""

        2 ->
            case b of
                Nothing ->
                    ""

        _ ->
            ""

