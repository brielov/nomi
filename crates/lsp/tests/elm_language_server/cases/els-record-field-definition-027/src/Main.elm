
infix left  0 (|>) = apR
apR : a -> (a -> b) -> b
apR x f = f x
type alias R = { field : () }
                  --X
type alias Outer r = { r : Type (r -> r) }
type Type a = Type a
foo : Outer r -> Outer r
foo r = r
main : Outer R
main =
    { r = Type (\r -> { r | field = () }) } |> foo
                             --^
