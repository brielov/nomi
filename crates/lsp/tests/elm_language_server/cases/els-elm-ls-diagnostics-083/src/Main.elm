
module Foo exposing (Patch(..))

type alias InputFiles =
    List String

type Patch
    = OnFiles InputFiles
			