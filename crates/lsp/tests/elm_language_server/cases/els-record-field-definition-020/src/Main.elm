
type alias R = { field : () }
                 --X
main : R -> ()
main { field } = field
       --^
