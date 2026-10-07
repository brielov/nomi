type Shape a
    = Circle Float
    | Rect { w : Float, h : Float } (List a)
    | Named Maybe.Maybe String
    | Empty
