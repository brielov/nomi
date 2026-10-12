module FooExtra exposing (..)

import Module exposing (foo)
                       --X

func = Module.foo
             --X

func2 = foo
       --X

