foo =
    let
        func = ""
       --^

        bar = func
              --X

        val = ""
    in
    ""

test = func

