
port module Ports exposing (..)
port foo : String -> Cmd msg
--X
update msg model = (model, foo "blah")
                           --^
