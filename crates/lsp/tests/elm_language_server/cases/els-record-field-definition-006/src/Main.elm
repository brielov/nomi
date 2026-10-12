
type alias R a = { field : a }
                 --X
main : R () -> ()
main r = r.field
           --^
