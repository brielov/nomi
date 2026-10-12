
module Foo exposing (..)

import List.Extra exposing ((!!))

getItemAtIndex : Int -> Maybe String
getItemAtIndex index =
    let
        someList =
            [ "a", "b", "c" ]
    in
    (!!) someList index
            