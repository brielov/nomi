port module LegacyPorts exposing (..)

port out : Int -> Cmd msg

port tasks =
    Signal.map f inputs
