module Main exposing (main)

{-| Imports of one module merge, but elm-format keeps one alias, so a
qualifier the module uses can go: here `Html.text` and `Pointer.Event`.
nomi keeps such an import apart. Merging the ones that lose nothing used
(`Dict`, `D`) stays elm-format's.
-}

import Dict as D exposing (Dict)
import Html
import Html as H exposing (div)
import Html.Events.Extra.Pointer as Pointer
import Html.Events.Extra.Pointer as Ptr exposing (onDown)
import Json.Decode as Decode exposing (int)


main : Html.Html msg
main =
    div [] [ Html.text (Debug.toString ( D.empty, Pointer.Event, int, Decode.int )) ]
