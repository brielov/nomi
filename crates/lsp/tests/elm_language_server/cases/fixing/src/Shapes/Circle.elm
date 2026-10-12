module Shapes.Circle exposing (Circle, area, circle)


type alias Circle =
    { radius : Float }


circle : Float -> Circle
circle radius =
    { radius = radius }


area : Circle -> Float
area c =
    pi * c.radius * c.radius
