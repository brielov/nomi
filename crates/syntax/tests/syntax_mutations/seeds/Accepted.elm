module Accepted exposing (answer, empty, identity)

import Dict as Dictionary exposing (Dict)


identity : value -> value
identity value =
    value


answer : Int
answer =
    identity 40 + 2


empty : Dictionary.Dict comparable value
empty =
    Dictionary.empty
