
type alias R = { field : () }
                 --X
main : R -> R
main r = { r | field = ()}
                --^
