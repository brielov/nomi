
type T = T { field : () }
              --X
main : T -> ()
main t =
     case t of
         T record ->
             record.field
                     --^
