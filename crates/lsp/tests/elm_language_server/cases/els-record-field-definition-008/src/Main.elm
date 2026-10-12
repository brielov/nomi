
type alias S = { nested : () }
                 --X
type alias R a = { field : a }
main : R S -> ()
main r = r.field.nested
                  --^
