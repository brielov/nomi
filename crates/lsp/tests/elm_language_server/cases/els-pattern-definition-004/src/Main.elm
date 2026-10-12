
f x =
    case () of
        Just foo -> ()
        _ -> foo
             --^unresolved
