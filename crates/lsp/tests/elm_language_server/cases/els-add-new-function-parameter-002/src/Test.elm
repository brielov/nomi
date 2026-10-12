module Test exposing (..)


outer : Int
outer =
    maybeToIntOrZero (Just "one")

