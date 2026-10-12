
module Bar exposing (text)

type Language
    = Italian (List String)
    | English (List String)


text : String -> String -> Html msg
text en it language =
    Html.text <|
        case language of
            English items ->
                en

            Italian items ->
                it
            