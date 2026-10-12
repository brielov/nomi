module Simple exposing (..)

import Expect
import Test exposing (..)


topSuite : Test
topSuite =
    describe "top suite" [ test "a test" <| \_ -> Expect.equal False False ]


someThingElse =
    True


nested : Test
nested =
    describe "outer"
        [ test "first" <| \_ -> Expect.equal True True
        , describe "nested"
            [ test "second" <| \_ -> Expect.equal False False
            ]
        , describe "empty" []
        ]
