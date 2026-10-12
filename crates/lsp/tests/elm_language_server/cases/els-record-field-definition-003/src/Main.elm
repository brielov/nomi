
type alias S = { nested : () }
type alias R = { field : S }
                 --X
main : R -> ()
main r = r.field.nested
           --^
