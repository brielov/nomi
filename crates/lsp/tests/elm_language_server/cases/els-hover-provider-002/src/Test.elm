module Test exposing (..)
import Models exposing (Msg(..))

update msg =
    case msg of
        Selected person ->
            person
          --^

