module Main exposing (..)

type Status = Ready
type alias CurrentStatus = Status

status : CurrentStatus
status = Ready
view = status

