
foo : a -> ()
foo _ =
    let
        bar : b -> b
            --X  --^
        bar b = b
    in
        ()
