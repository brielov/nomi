module Shapes.Circle exposing (Circle, area)


type alias Circle =
    { radius : Float }


area : Circle -> Float
area circle =
    pi * circle.radius * circle.radius
