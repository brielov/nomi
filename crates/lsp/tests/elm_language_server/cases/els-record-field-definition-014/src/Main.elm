
infix left  0 (|>) = apR
apR : a -> (a -> b) -> b
apR x f = f x
type alias R = { field : () }
                 --X
func : R -> ()
func _ = ()
main : ()
main = { field = () } |> func
          --^
