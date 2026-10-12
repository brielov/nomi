module Test exposing (..)

type alias Model =
  { prop1: String
  , prop2: Int
  }

type Maybe a = Just a | Nothing

testFunction : Maybe Model -> a
testFunction model =
    case model of
        Just m ->
            m.

