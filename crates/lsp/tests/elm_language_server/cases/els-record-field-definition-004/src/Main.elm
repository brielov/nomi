
type alias R = { field : () }
                 --X
main : R -> ()
main r =
    .field r
      --^
