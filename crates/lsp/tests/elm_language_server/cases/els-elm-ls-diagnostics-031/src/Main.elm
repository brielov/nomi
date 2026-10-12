
module Bar exposing (..)

import Foo exposing (empty)

addUsedVariable =
    { empty | name = "John" }
			