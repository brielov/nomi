module Module exposing (Foo)

type alias Foo = { field : String }
                   --^

foo : Foo -> String
foo arg  =
    arg.field
        --X

update : Foo -> Foo
update arg =
    { arg | field = "" }
            --X

test : Foo
test =
    { field = "" }
      --X

destructure : Foo -> String
destructure { field } =
              --X
    field
    --X

