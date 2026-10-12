module Test exposing (Type, t)

type alias Type =
    { tag : String
    , data : Int
    }

t : Type
  --^
t =
    {}

