
infix right 0 (<|) = apL
apL : (a -> b) -> a -> b
apL f x =
  f x
type alias R = { field : () }
                 --X
func : R -> ()
func _ = ()
main : ()
main = func <| { field = () }
                 --^
