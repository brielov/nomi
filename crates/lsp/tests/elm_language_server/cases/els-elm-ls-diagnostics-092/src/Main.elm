
module Bar exposing (text)

type Language
    = Italian
    | English


text : String -> String -> Html msg
text en it language =
    Html.text <|
        case language of
            English ->
                en

            Italian ->
                it
			