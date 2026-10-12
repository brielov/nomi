
type alias R = { field : () }
                 --X
main : R
main = (\_ -> { field = () }) 1
                 --^
