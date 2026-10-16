module Todo exposing (Todo, encode, decoder, encodeTitle, titleDecoder)

{-| A todo as the api sends it and the web app reads it. Both apps use
these codecs, so the JSON between them can't drift apart.

@docs Todo, encode, decoder, encodeTitle, titleDecoder

-}

import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode


{-| A todo.
-}
type alias Todo =
    { id : Int
    , title : String
    , done : Bool
    }


{-| A todo as JSON.
-}
encode : Todo -> Encode.Value
encode todo =
    Encode.object
        [ ( "id", Encode.int todo.id )
        , ( "title", Encode.string todo.title )
        , ( "done", Encode.bool todo.done )
        ]


{-| A todo from the JSON `encode` writes.
-}
decoder : Decoder Todo
decoder =
    Decode.map3 Todo
        (Decode.field "id" Decode.int)
        (Decode.field "title" Decode.string)
        (Decode.field "done" Decode.bool)


{-| The body that asks for a new todo: `{"title": "milk"}`.
-}
encodeTitle : String -> Encode.Value
encodeTitle title =
    Encode.object [ ( "title", Encode.string title ) ]


{-| The title in the body `encodeTitle` writes.
-}
titleDecoder : Decoder String
titleDecoder =
    Decode.field "title" Decode.string
