module Test exposing (..)
type Status = Ready
type Msg = Selected Status
select status = Selected status
                       --^

