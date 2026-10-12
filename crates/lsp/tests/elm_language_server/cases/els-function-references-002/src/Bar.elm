module Bar exposing (..)

import Module exposing (foo)
                       --X

bar = foo 1 2
     --X

