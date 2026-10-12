
type alias R =
  { field : () }
    --X

type Box a
  = Box a

map : (a -> b) -> Box a -> Box b
map f (Box a) =
    Box (f a)

main : Box R -> Box R
main box =
    let
        f r =
          { r | field = () }
                --^
    in
    map f box
