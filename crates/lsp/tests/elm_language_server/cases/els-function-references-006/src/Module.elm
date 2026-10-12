module Module exposing (foo)
                        --X
foo = 42
--^

bar = foo
     --X

