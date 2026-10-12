module Models exposing (Box, box)

type Status = Ready
type alias Box = { value : Status }

box : Box
box = { value = Ready }

