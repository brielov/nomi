module Basics exposing (..)

infix right0 (<|) = apL
infix non 4 (==) = eq

apL f x = f x
