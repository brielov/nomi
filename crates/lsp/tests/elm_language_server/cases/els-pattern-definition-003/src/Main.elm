
type Foo = Foo
           --X
f x =
    case x of
        Nothing -> 0
        Just Foo -> 1
             --^
