module Test exposing (..)


type A
   --^
    = A


type Aardvark
    = Aardvark

--@ Test2.elm

module Test2 exposing (..)

import Test exposing (Aardvark)


test =
    5



