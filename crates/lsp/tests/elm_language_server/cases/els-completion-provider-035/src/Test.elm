module Test exposing (..)

type alias Model =
  { prop1: String
  , prop2: Int
  }

map: (a -> b) -> List a -> List b
map func list =
  list

testFunction : List Model -> List a
testFunction model =
    map (\m -> m.) model

