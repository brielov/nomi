
type alias R = { field : () }
                 --X
main : R -> ()
main r =
  let
    nest = r.field
             --^
  in
  nest
