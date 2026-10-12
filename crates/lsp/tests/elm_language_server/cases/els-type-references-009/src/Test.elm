module Test exposing (..)

type MyRecord =
      --^
    MyRecord Int


decodeStuff : MyRecord
              --X
decodeStuff =
    MyRecord 0

