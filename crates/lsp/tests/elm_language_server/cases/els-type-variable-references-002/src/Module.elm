module Module exposing (..)

foo : Model a -> a
          --^  --X
foo = 
    let
        func : a -> Model a
             --X        --X
        func =
            32

    in
    42

