module Test exposing (..)

type Foo
    = Bar
    | Biz

test : Foo -> Html.Html msg
test response =
    case response of
        Bar ->
            Html.text "x"

