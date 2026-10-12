
type alias R a = { a | field : () }
type alias S = { s : R { field2 : () } }
                          --X
main : S -> ()
main r = r.s.field2
               --^
