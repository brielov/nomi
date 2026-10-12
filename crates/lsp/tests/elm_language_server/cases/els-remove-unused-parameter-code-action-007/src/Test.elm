module Test exposing (..)

foo : Maybe Int -> Int -> Int
foo (Just x) unused = x

result = foo (Just 1) 2

