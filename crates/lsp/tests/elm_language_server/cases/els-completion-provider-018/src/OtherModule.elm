module OtherModule exposing (testFunction, TestType, testInPort, testOutPort)

testFunction : String
testFunction =
  ""

type alias TestType = {prop : String}

port testOutPort : String -> Cmd msg

port testInPort : (String -> msg) -> Sub msg

