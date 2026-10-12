
module Bar exposing (foo)

test =
    let
        aList =
            if True then
                [ 2, 3 ]

            else
                []
    in
    List.concat
        [ [ 1 ]
        , aList
        , [ 4 ]
        ]
			