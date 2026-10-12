
type alias R = { field : () }
                 --X
func : R -> ()
func _ = ()
main : ()
main = func { field = () }
               --^
