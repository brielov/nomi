module Shapes.Circle exposing (Circle, area, unit)


type alias Circle =
    { radius : Float }


unit : Circle
unit =
    { radius = 1 }


area : Circle -> Float
area circle =
    pi * circle.radius * circle.radius
