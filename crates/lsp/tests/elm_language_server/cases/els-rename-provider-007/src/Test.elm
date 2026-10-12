module Test exposing (..)

type alias Foo =
  { foo : Bar
  }


type alias Bar =
  { bar : Int }
  --^


getBar : Foo -> Int
getBar value =
  value.foo.bar


incrementBar : Foo -> Int
incrementBar value =
  value.foo.bar + 1

