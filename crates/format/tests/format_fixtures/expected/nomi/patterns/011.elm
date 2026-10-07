module Example exposing (x)


x value =
    case value of
        (head {- A -} :: {- B -} tail) {- C -} as {- D -} whole ->
            ()
