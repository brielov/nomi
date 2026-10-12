port module Module exposing (..)
testFunc = ""

type Msg = Msg1 | Msg2

type alias Model = { field : String }

port testPort : String -> Cmd msg

