module M exposing (..)

(=>) : a -> b -> ( a, b )
(=>) a b =
    ( a, b )

(-) =
    let
        (::) x = x
    in
    sub
