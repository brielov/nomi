module Test exposing (..)

foo : Int -> Int
foo unused = 1

result = foo (foo 2)

