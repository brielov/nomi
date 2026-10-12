module Test exposing (..)

type alias Model =
  { field : Int
	}

func : Model -> { prop : Int }
func model =
		model.newProp

