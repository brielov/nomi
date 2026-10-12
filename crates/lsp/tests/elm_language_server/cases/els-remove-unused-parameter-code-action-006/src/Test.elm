module Test exposing (..)

foo unused x = if x == 0 then x else foo 2 0

result = foo 1 3

