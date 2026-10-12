
type alias R = { field : () }
                 --X
main : R -> ()
main r =
  case r of
      { field } -> field
        --^
