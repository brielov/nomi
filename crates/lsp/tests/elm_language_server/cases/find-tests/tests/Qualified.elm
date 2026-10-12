module Qualified exposing (..)

import Expect
import Fuzz
import Test
import Test as T


plain : Test.Test
plain =
    Test.describe "qualified suite" [ Test.test "a test" <| \_ -> Expect.equal 1 1 ]


aliased : T.Test
aliased =
    T.describe "aliased suite" [ T.fuzz Fuzz.int "a fuzz" <| \n -> Expect.equal n n ]


label : String
label =
    "dynamic"


dynamic : T.Test
dynamic =
    T.test label <| \_ -> Expect.pass


multiline : T.Test
multiline =
    T.test """a "multiline"
label""" <| \_ -> Expect.pass


escaped : T.Test
escaped =
    T.test "an \"escaped\" \t label" <| \_ -> Expect.pass
