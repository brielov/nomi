module AcceptedTypes exposing (Maybe(..), Pair)


type Maybe value
    = Nothing
    | Just value


type alias Pair left right =
    { left : left, right : right }
