
type alias R = { field : () }
                 --X
main : List R
main =
    [ { field = () }
    , { field = () }
        --^
    ]
