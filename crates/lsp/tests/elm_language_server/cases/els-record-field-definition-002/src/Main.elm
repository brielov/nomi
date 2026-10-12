
type alias S = { nested : () }
                 --X
type alias R = { field : S }
main : R -> ()
main r = r.field.nested
                  --^
