module Main exposing (apply)


apply f =
    let
        g x =
            f x
    in
    g
