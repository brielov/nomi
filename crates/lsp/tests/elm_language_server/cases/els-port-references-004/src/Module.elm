port module Module exposing (foo)
                            --X
port foo : String -> Cmd msg
    --^

bar = foo
     --X

