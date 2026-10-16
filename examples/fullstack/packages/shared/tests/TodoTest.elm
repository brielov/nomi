module TodoTest exposing (suite)

import Expect
import Fuzz
import Json.Decode as Decode
import Json.Encode as Encode
import Test exposing (Test, describe, fuzz3, test)
import Todo exposing (Todo)


suite : Test
suite =
    describe "Todo"
        [ fuzz3 Fuzz.int Fuzz.string Fuzz.bool "decodes what it encodes" <|
            \id title done ->
                Todo id title done
                    |> Todo.encode
                    |> Decode.decodeValue Todo.decoder
                    |> Expect.equal (Ok (Todo id title done))
        , test "is an object of its fields" <|
            \_ ->
                Todo 1 "milk" False
                    |> Todo.encode
                    |> Encode.encode 0
                    |> Expect.equal """{"id":1,"title":"milk","done":false}"""
        , test "a new todo's title" <|
            \_ ->
                Todo.encodeTitle "milk"
                    |> Decode.decodeValue Todo.titleDecoder
                    |> Expect.equal (Ok "milk")
        ]
