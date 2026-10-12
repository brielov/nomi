
foo x =
--X
    if x <= 0 then 0 else foo (x - 1)
                          --^
