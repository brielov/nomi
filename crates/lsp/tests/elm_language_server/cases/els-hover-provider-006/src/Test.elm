module Test exposing (..)

type alias Foo = 
    String

foo : Foo
foo = ""

bar = foo
     --^
    
