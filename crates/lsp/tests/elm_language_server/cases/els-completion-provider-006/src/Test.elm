module Test exposing (..)

type alias Model =
  { prop1: String
  , prop2: Int
  }

view : Model -> Model
view model =
  let
    var : String
    var =
      model.
        |> String.toFloat
        |> String.fromFloat

  in
    model

