module Models exposing (Person, Msg(..))

type alias Person = { firstname : String }
type Msg = Selected Person

