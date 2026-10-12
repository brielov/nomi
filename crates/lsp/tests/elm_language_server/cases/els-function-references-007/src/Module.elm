module Module exposing (foo)
                        --X
foo a = 42
--^

bar = foo 1
     --X

