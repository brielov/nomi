module M exposing (..)

(<$>) : (a -> b) -> Maybe a -> Maybe b
(<$>) =
    Maybe.map

x : Int → Int
x =
    \n → f <$> n ⊕ n |~ n € n
