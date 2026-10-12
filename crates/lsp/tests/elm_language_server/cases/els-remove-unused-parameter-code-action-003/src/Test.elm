module Test exposing (..)

foo unused x = x

partial = (foo) 1
result = 2 |> foo 3
higherOrder = (\f -> f 2) (foo 4)

