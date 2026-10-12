module Lets exposing (..)

import Expect
import Test exposing (..)


withLet : Test
withLet =
    let
        value =
            13
    in
    let
        other =
            value
    in
    describe "let suite" [ test "a test" <| \_ -> Expect.equal other value ]


deep : Test
deep =
    describe "top suite"
        [ let
            a =
                13
          in
          describe "deeper suite" [ test "a test" <| \_ -> Expect.equal a a ]
        ]
