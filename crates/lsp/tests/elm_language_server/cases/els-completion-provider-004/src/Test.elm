module Test exposing (..)

type alias Model =
  { prop1: Data
  , prop2: Int
  }

type alias Data =
  { item1: String
  , item2: Int
  }

view : Model -> Model
view model =
  { model | prop1 = { i } }

