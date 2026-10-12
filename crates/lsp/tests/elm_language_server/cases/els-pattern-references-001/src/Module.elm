module Module exposing (foo)

foo =
    let
        ( first, second ) = func
          --^
    in
    case first of
         --X
        "" ->
            bar first
                --X

        _ ->
            ""

