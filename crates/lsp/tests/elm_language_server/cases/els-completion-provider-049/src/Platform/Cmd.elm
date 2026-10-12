module Platform.Cmd exposing (Cmd, batch)

type Cmd msg = Cmd

batch : List (Cmd msg) -> Cmd msg
batch =
  Elm.Kernel.Platform.batch

