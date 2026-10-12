module Test exposing (..)

result =
    let
        foo unused x = x
    in
    foo 1 2

other =
    let
        foo a b = a + b
    in
    foo 3 4

