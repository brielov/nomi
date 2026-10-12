
foo : Int -> Int
--^
foo a = a
--X
outer =
    let
        foo a = a
    in foo
