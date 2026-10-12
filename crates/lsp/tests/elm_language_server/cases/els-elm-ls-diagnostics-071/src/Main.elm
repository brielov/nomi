
module Bar exposing (foo)

foo x =
  case x of
    DropConsOfItemAndList fileName range ->
        ( String.concat
            [ "Adding an item to the front of a literal list, but instead you can just put it in the list. "
            , fileName
            , " at "
            , rangeToString range
            ]
        , always (List.concat [ [ fileName ], [] ])
        , [ range ]
        , True
        )
            