module Foo exposing (..)

import Test exposing (MyRecord)
                      --X

decodeStuff : MyRecord
              --X
decodeStuff =
    MyRecord 0
    --X

