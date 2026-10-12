
type alias R = { field : () }
                 --X
main : R -> ()
main r = (\rr -> rr.field) r
                     --^
