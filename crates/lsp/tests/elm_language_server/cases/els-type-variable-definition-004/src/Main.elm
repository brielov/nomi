
foo : a -> { f : a } -> ()
    --X        --^
foo _ _ = ()
