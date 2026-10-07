module Example exposing (X)


type alias X a =
    { a | x : { b | y : Int } }
