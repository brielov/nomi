module Main exposing (main)

{-| The todo list in the browser. It reads and adds todos through the api,
whose address `index.html` passes as a flag, with acme/shared's codecs.
-}

import Browser
import Html exposing (Html)
import Html.Attributes as Attr
import Html.Events as Events
import Http
import Json.Decode as Decode
import Todo exposing (Todo)


type alias Flags =
    { api : String }


type alias Model =
    { api : String
    , todos : List Todo
    , title : String
    , problem : Maybe String
    }


type Msg
    = GotTodos (Result Http.Error (List Todo))
    | Added (Result Http.Error Todo)
    | TitleChanged String
    | Submitted


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , view = view
        , update = update
        , subscriptions = \_ -> Sub.none
        }


init : Flags -> ( Model, Cmd Msg )
init flags =
    ( { api = flags.api, todos = [], title = "", problem = Nothing }
    , Http.get
        { url = flags.api ++ "/todos"
        , expect = Http.expectJson GotTodos (Decode.list Todo.decoder)
        }
    )


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotTodos (Ok todos) ->
            ( { model | todos = todos, problem = Nothing }, Cmd.none )

        Added (Ok todo) ->
            ( { model | todos = model.todos ++ [ todo ], title = "", problem = Nothing }, Cmd.none )

        GotTodos (Err error) ->
            ( { model | problem = Just (describe error) }, Cmd.none )

        Added (Err error) ->
            ( { model | problem = Just (describe error) }, Cmd.none )

        TitleChanged title ->
            ( { model | title = title }, Cmd.none )

        Submitted ->
            ( model
            , Http.post
                { url = model.api ++ "/todos"
                , body = Http.jsonBody (Todo.encodeTitle model.title)
                , expect = Http.expectJson Added Todo.decoder
                }
            )


describe : Http.Error -> String
describe error =
    case error of
        Http.BadStatus status ->
            "The api answered " ++ String.fromInt status ++ "."

        Http.BadBody problem ->
            "The api's answer wasn't a todo: " ++ problem

        _ ->
            "The api didn't answer. Is it running?"


view : Model -> Html Msg
view model =
    Html.div []
        [ Html.h1 [] [ Html.text "Todos" ]
        , Html.ul [] (List.map viewTodo model.todos)
        , Html.form [ Events.onSubmit Submitted ]
            [ Html.input [ Attr.value model.title, Events.onInput TitleChanged, Attr.placeholder "Something to do" ] []
            , Html.button [] [ Html.text "Add" ]
            ]
        , case model.problem of
            Just problem ->
                Html.p [] [ Html.text problem ]

            Nothing ->
                Html.text ""
        ]


viewTodo : Todo -> Html msg
viewTodo todo =
    Html.li [ Attr.classList [ ( "done", todo.done ) ] ] [ Html.text todo.title ]
