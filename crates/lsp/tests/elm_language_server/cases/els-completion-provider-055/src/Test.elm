module Test exposing (..)

type alias Foo = { prop : String }

type alias Model =
  { prop1: String
  , prop2: Int
  }

view : Foo -> Model -> String
view { prop } model =
    let
        x = model.p
    in
    ""

