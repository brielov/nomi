module Combined exposing (Combined, fields)

import Left exposing (Left)
import Right exposing (Right)

type alias Combined =
    { left : Left
    , right : Right
    }

fields : Combined -> ( String, String )
fields combined =
    ( combined.left.foo.field
                        --X
    , combined.right.foo.field
                         --X
    )

