module Test exposing (..)

foo : (Int -> Int) -> Int -> Int
foo unused x = x

result =
    foo
        (\x -> x)
        2

