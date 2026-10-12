module Main exposing (main)

import Html exposing (Html, text)


main : Html msg
main =
    text (String.fromInt (lenght "elm"))
                         --^


count : Int
count =
    "three"
  --^
