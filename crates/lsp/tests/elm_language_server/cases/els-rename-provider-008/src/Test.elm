module Test exposing (..)

type alias Foo =
  { foo : Bar
  }


type alias Bar =
  { bar : Int }
  --^


destructureBar : Foo -> Int
destructureBar { foo } =
  let
    { bar } = foo
  in
  bar


destructureBarAgain : Foo -> Int
destructureBarAgain { foo } =
  let
    { bar } = foo
  in
  bar + 1

