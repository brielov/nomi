
type alias Person a = { name : String, extra : a }
--X
title : Person a -> String
        --^
