port module Test exposing (..)

x = 
    f

port foo : String -> Cmd msg

port fbar : (String -> msg) -> Sub msg

