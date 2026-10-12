
foo : b -> a -> ()
         --X
foo _ =
    let
        bar : a -> { f : { g : a } }
                             --^
        bar b = b
    in
        ()
