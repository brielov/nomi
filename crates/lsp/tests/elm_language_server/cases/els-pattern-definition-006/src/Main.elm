
f x =
    case () of
        _ ->
            case () of
                Just foo -> ()
                _ -> foo
                     --^unresolved
