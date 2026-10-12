
type alias R = { field : () }
                 --X
main : R -> ()
main r = { r | field = () }.field
                           --^
