
port module Bar exposing (foo)

bar = 2

type alias Foo = {}

type Bar = Other

port foo : String -> Cmd msg
			