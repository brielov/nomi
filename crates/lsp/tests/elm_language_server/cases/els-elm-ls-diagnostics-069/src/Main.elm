
module Bar exposing (foo)

type alias Params x a =
    { x
        | id : String
        , label : String
        , action : a
    }
-- more type aliasses that extends from Params
type alias CheckboxParams a =
    Params { checked : Bool } (Bool -> a)
			