
foo : a -> (a -> a) -> ()
    --X        --^
foo _ _ = ()
