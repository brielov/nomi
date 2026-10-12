
foo =
    let
        bar y = 0
        quux = y
             --^unresolved
    in
        quux
