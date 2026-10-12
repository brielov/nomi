module Main exposing (main)

import Html exposing (Html, div, text)


type alias Model =
    { count : Int
    , name : String
    }


type Msg
    = Increment
    | Rename String


init : Model
init =
    { count = 0, name = "" }


update : Msg -> Model -> Model
update msg model =
    case msg of
        Increment ->
            { model | count = model.count + 1 }
                                    --^

        Rename name ->
            { model | name = name }


-- region helpers


twice : Int -> Int
--^
twice n =
    n * 2


-- endregion


main : Html msg
main =
    div [] [ text (String.fromInt (twice init.count)) ]
             --^        --^
