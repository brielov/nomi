module Test exposing (..)

type alias MyRecord =
            --^
    { score : Int
    }


decodeStuff : MyRecord
              --X
decodeStuff =
    MyRecord 0
    --X

