module Test exposing (State(..), t)

type State
    = Ok
    | Average
    | Bad

t : State
t =
    Ok
  --^

