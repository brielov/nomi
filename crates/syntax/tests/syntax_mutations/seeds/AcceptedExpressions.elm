module AcceptedExpressions exposing (Maybe(..), apply, choose, describe, increment, names, swap)


type Maybe value
    = Nothing
    | Just value


choose predicate yes no =
    if predicate then
        yes
    else
        no


describe maybe =
    case maybe of
        Just value ->
            value

        Nothing ->
            "none"


increment model =
    { model | count = model.count + 1 }


apply fn value =
    (\argument -> fn argument) value


names models =
    List.map .name models


swap pair =
    let
        ( left, right ) = pair
    in
    ( right, left )
