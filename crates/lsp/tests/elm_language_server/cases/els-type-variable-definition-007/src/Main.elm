
foo : a -> ()
    --X
foo _ =
    let
        bar : a -> a -> ()
                 --^
        bar b = b
    in
        ()
