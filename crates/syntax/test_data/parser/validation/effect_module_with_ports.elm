effect module Task where { command = MyCmd, subscription = MySub } exposing (..)

port send : String -> Cmd msg
