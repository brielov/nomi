
type alias User = { name : String, age : Int }

foo user =
  case user of
    User -> "foo"
    --^unresolved
    