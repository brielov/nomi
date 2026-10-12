
type User = GroupA

foo user =
  case user of
    User -> "foo"
    --^unresolved
    