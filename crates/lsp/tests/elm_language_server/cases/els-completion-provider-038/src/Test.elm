module Test exposing (..)

type alias Model =
  { prop1: Model2
  , prop2: Int
  }

type alias Model2 =
  { field1: String
  , field2: Int }

type Maybe a = Just a | Nothing

testFunction : Maybe Model -> a
testFunction model =
    case model of
        Just { prop1, prop2 } ->
            prop1.f

        Nothing ->
            ""

