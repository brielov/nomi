import App exposing (..)

fooLens =
    FieldLens .foo (\newFoo r -> { r | foo = newFoo })
    --^App.elm

