
type alias R = { field : () }
                 --X
r : () -> R
r unit = { field = unit }
main = (r ()).field
               --^
