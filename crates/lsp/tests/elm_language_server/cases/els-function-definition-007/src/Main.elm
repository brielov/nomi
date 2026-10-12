
foo =
    let
        bar y = if y <= 0 then 0 else bar (y - 1)
        --X                           --^
    in bar 100
