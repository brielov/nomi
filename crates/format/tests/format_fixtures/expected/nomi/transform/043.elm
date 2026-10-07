module Main exposing (runner)


port {- A -} runner {- B -} : {- C -} Signal (Task.Task x ())
port {- A -} runner {- B -} =
    -- C
    Signal.constant (Task.succeed ())
