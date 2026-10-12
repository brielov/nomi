module Test exposing (..)

import Another exposing (Foo)


foo : Foo
foo = 
  { bar = 10
  , biz = "Hello World"
   --^
  }
    
