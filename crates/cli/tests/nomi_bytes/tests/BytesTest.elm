module BytesTest exposing (suite)

import Bytes exposing (Bytes)
import Bytes.Decode as Decode
import Bytes.Encode as Encode
import Expect
import Fuzz exposing (Fuzzer)
import Nomi.Bytes
import Test exposing (Test, describe, fuzz, fuzz2, fuzz3, test)


suite : Test
suite =
    describe "Nomi.Bytes"
        [ againstElmBytes
        , strictUtf8
        , base64AndHex
        ]



-- AGAINST ELM/BYTES


againstElmBytes : Test
againstElmBytes =
    describe "against elm/bytes"
        [ fuzz bytes "length is Bytes.width" <|
            \b -> Nomi.Bytes.length b |> Expect.equal (Bytes.width b)
        , fuzz3 bytes index index "slice takes what String.slice would" <|
            \b start end -> toList (Nomi.Bytes.slice start end b) |> Expect.equal (listSlice start end (toList b))
        , fuzz3 bytes index index "a slice decodes as Decode.bytes reads it" <|
            \b i j ->
                let
                    ( start, end ) =
                        ( min (abs i) (abs j) |> min (Bytes.width b), max (abs i) (abs j) |> min (Bytes.width b) )

                    read =
                        Decode.map2 (\_ taken -> taken) (Decode.bytes start) (Decode.bytes (end - start))
                in
                Decode.decode read b
                    |> Maybe.map toList
                    |> Expect.equal (Just (toList (Nomi.Bytes.slice start end b)))
        , fuzz bytes "a slice of a slice is elm/bytes' to read and write" <|
            \b ->
                let
                    inner =
                        Nomi.Bytes.slice 1 (-1) (Nomi.Bytes.slice 1 (-1) b)
                in
                Encode.encode (Encode.sequence [ Encode.bytes inner, Encode.unsignedInt8 7 ])
                    |> toList
                    |> Expect.equal (listSlice 2 (-2) (toList b) ++ [ 7 ])
        , fuzz2 bytes bytes "append writes what Encode.sequence writes" <|
            \a b ->
                toList (Nomi.Bytes.append a b)
                    |> Expect.equal (toList (Encode.encode (Encode.sequence [ Encode.bytes a, Encode.bytes b ])))
        , fuzz (Fuzz.list bytes) "concat writes what Encode.sequence writes" <|
            \all ->
                toList (Nomi.Bytes.concat all)
                    |> Expect.equal (toList (Encode.encode (Encode.sequence (List.map Encode.bytes all))))
        , fuzz2 fewBytes fewBytes "equal is whether the bytes are the same" <|
            \a b -> Nomi.Bytes.equal a b |> Expect.equal (toList a == toList b)
        , fuzz bytes "equal sees the same bytes in a slice and a copy" <|
            \b ->
                Nomi.Bytes.equal (Nomi.Bytes.slice 1 (-1) b) (fromList (listSlice 1 (-1) (toList b)))
                    |> Expect.equal True
        , fuzz2 fewBytes fewBytes "indexOf finds where the bytes first appear" <|
            \needle haystack ->
                Nomi.Bytes.indexOf needle haystack
                    |> Expect.equal (listIndexOf (toList needle) (toList haystack))
        , fuzz3 bytes index index "indexOf finds a slice of the bytes searched" <|
            \b start end ->
                let
                    needle =
                        Nomi.Bytes.slice start end b
                in
                Nomi.Bytes.indexOf needle b
                    |> Expect.equal (listIndexOf (toList needle) (toList b))
        , fuzz text "fromString writes what Encode.string writes" <|
            \s -> toList (Nomi.Bytes.fromString s) |> Expect.equal (toList (Encode.encode (Encode.string s)))
        , fuzz text "toString reads what Encode.string wrote" <|
            \s ->
                let
                    b =
                        Encode.encode (Encode.string s)
                in
                Nomi.Bytes.toString b
                    |> Expect.all
                        [ Expect.equal (Just s)
                        , Expect.equal (Decode.decode (Decode.string (Bytes.width b)) b)
                        ]
        , fuzz bytes "toString reads UTF-8 as Decode.string does" <|
            \b ->
                case Nomi.Bytes.toString b of
                    Just s ->
                        Just s |> Expect.equal (Decode.decode (Decode.string (Bytes.width b)) b)

                    Nothing ->
                        Expect.pass
        , test "half a surrogate pair becomes U+FFFD" <|
            \_ ->
                toList (Nomi.Bytes.fromString (String.fromList [ Char.fromCode 0xD800, 'x', Char.fromCode 0xDC00 ]))
                    |> Expect.equal [ 0xEF, 0xBF, 0xBD, 0x78, 0xEF, 0xBF, 0xBD ]
        ]



-- STRICT UTF-8


strictUtf8 : Test
strictUtf8 =
    describe "strict UTF-8"
        [ describe "refuses"
            (List.map
                (\( name, sequence ) ->
                    test name <|
                        \_ ->
                            [ sequence, [ 0x61 ] ++ sequence ++ [ 0x62 ] ]
                                |> List.map (fromList >> Nomi.Bytes.toString)
                                |> Expect.equal [ Nothing, Nothing ]
                )
                refused
            )
        , describe "reads the characters at the edges of each length"
            (List.map
                (\( code, sequence ) ->
                    test (String.fromInt code) <|
                        \_ ->
                            Nomi.Bytes.toString (fromList sequence)
                                |> Expect.equal (Just (String.fromChar (Char.fromCode code)))
                )
                edges
            )
        , test "keeps a byte order mark" <|
            \_ ->
                Nomi.Bytes.toString (fromList [ 0xEF, 0xBB, 0xBF, 0x61 ])
                    |> Expect.equal (Just (String.fromList [ Char.fromCode 0xFEFF, 'a' ]))
        ]


refused : List ( String, List Int )
refused =
    [ ( "a continuation byte with no start", [ 0x80 ] )
    , ( "a byte no character starts with", [ 0xFF ] )
    , ( "a five-byte start", [ 0xF8, 0x88, 0x80, 0x80, 0x80 ] )
    , ( "a character cut short at the end", [ 0xE2, 0x82 ] )
    , ( "a character cut short by another", [ 0xE2, 0x28, 0xA1 ] )
    , ( "an ASCII character in two bytes", [ 0xC1, 0xBF ] )
    , ( "a two-byte character in three", [ 0xE0, 0x9F, 0xBF ] )
    , ( "a three-byte character in four", [ 0xF0, 0x8F, 0xBF, 0xBF ] )
    , ( "a high surrogate", [ 0xED, 0xA0, 0x80 ] )
    , ( "a low surrogate", [ 0xED, 0xBF, 0xBF ] )
    , ( "a surrogate pair, each half in three bytes", [ 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80 ] )
    , ( "a character past U+10FFFF", [ 0xF4, 0x90, 0x80, 0x80 ] )
    ]


edges : List ( Int, List Int )
edges =
    [ ( 0x7F, [ 0x7F ] )
    , ( 0x80, [ 0xC2, 0x80 ] )
    , ( 0x07FF, [ 0xDF, 0xBF ] )
    , ( 0x0800, [ 0xE0, 0xA0, 0x80 ] )
    , ( 0xD7FF, [ 0xED, 0x9F, 0xBF ] )
    , ( 0xE000, [ 0xEE, 0x80, 0x80 ] )
    , ( 0xFFFF, [ 0xEF, 0xBF, 0xBF ] )
    , ( 0x00010000, [ 0xF0, 0x90, 0x80, 0x80 ] )
    , ( 0x0010FFFF, [ 0xF4, 0x8F, 0xBF, 0xBF ] )
    ]



-- BASE64 AND HEX


base64AndHex : Test
base64AndHex =
    describe "base64 and hex"
        [ test "RFC 4648's examples" <|
            \_ ->
                [ "", "f", "fo", "foo", "foob", "fooba", "foobar" ]
                    |> List.map (\s -> ( Nomi.Bytes.toBase64 (Nomi.Bytes.fromString s), Nomi.Bytes.toHex (Nomi.Bytes.fromString s) ))
                    |> Expect.equal
                        [ ( "", "" )
                        , ( "Zg==", "66" )
                        , ( "Zm8=", "666f" )
                        , ( "Zm9v", "666f6f" )
                        , ( "Zm9vYg==", "666f6f62" )
                        , ( "Zm9vYmE=", "666f6f6261" )
                        , ( "Zm9vYmFy", "666f6f626172" )
                        ]
        , test "the alphabets differ in two characters" <|
            \_ ->
                fromList [ 0xFB, 0xFF, 0x01 ]
                    |> Expect.all
                        [ Nomi.Bytes.toBase64 >> Expect.equal "+/8B"
                        , Nomi.Bytes.toBase64Url >> Expect.equal "-_8B"
                        , Nomi.Bytes.toHex >> Expect.equal "fbff01"
                        ]
        , fuzz bytes "toBase64Url is toBase64 in the other alphabet, unpadded" <|
            \b ->
                Nomi.Bytes.toBase64Url b
                    |> Expect.equal (Nomi.Bytes.toBase64 b |> String.replace "+" "-" |> String.replace "/" "_" |> String.replace "=" "")
        , fuzz bytes "base64 round-trips, padded or not, in either alphabet" <|
            \b ->
                [ Nomi.Bytes.toBase64 b, String.replace "=" "" (Nomi.Bytes.toBase64 b), Nomi.Bytes.toBase64Url b ]
                    |> List.map (Nomi.Bytes.fromBase64 >> Maybe.map toList)
                    |> Expect.equal (List.repeat 3 (Just (toList b)))
        , fuzz bytes "hex round-trips in either case" <|
            \b ->
                [ Nomi.Bytes.toHex b, String.toUpper (Nomi.Bytes.toHex b) ]
                    |> List.map (Nomi.Bytes.fromHex >> Maybe.map toList)
                    |> Expect.equal (List.repeat 2 (Just (toList b)))
        , fuzz bytes "hex is two digits a byte" <|
            \b ->
                Nomi.Bytes.toHex b
                    |> Expect.equal (String.concat (List.map hexByte (toList b)))
        , test "fromBase64 refuses what toBase64 and toBase64Url don't write" <|
            \_ ->
                [ "Zg=", "Zg===", "=Zg=", "Z", "Zh==", "Zm9", "Zm9v\n", "Zm9v YmFy", "Zm+v_w", "Zm9v!" ]
                    |> List.filterMap (\s -> Nomi.Bytes.fromBase64 s |> Maybe.map (\_ -> s))
                    |> Expect.equal []
        , test "fromHex refuses what isn't hex" <|
            \_ ->
                [ "abc", "zz", "0x00", "ab cd", "ab\n" ]
                    |> List.filterMap (\s -> Nomi.Bytes.fromHex s |> Maybe.map (\_ -> s))
                    |> Expect.equal []
        ]



-- HELPERS


bytes : Fuzzer Bytes
bytes =
    Fuzz.listOfLengthBetween 0 100 (Fuzz.intRange 0 255) |> Fuzz.map fromList


{-| Bytes from three values, so that one is often found in another.
-}
fewBytes : Fuzzer Bytes
fewBytes =
    Fuzz.listOfLengthBetween 0 8 (Fuzz.intRange 0 2) |> Fuzz.map fromList


index : Fuzzer Int
index =
    Fuzz.intRange (-110) 110


{-| Text of every length of UTF-8 character, and no half surrogate pair.
-}
text : Fuzzer String
text =
    Fuzz.oneOf
        [ Fuzz.intRange 0 0x7F
        , Fuzz.intRange 0x80 0x07FF
        , Fuzz.intRange 0x0800 0xD7FF
        , Fuzz.intRange 0xE000 0xFFFF
        , Fuzz.intRange 0x00010000 0x0010FFFF
        ]
        |> Fuzz.map Char.fromCode
        |> Fuzz.list
        |> Fuzz.map String.fromList


fromList : List Int -> Bytes
fromList ints =
    Encode.encode (Encode.sequence (List.map Encode.unsignedInt8 ints))


toList : Bytes -> List Int
toList b =
    let
        step ( left, taken ) =
            if left <= 0 then
                Decode.succeed (Decode.Done (List.reverse taken))

            else
                Decode.map (\byte -> Decode.Loop ( left - 1, byte :: taken )) Decode.unsignedInt8
    in
    Decode.decode (Decode.loop ( Bytes.width b, [] ) step) b |> Maybe.withDefault []


{-| `String.slice` on a list.
-}
listSlice : Int -> Int -> List a -> List a
listSlice start end list =
    let
        n =
            List.length list

        at i =
            if i < 0 then
                max 0 (n + i)

            else
                min n i
    in
    list |> List.drop (at start) |> List.take (at end - at start)


listIndexOf : List Int -> List Int -> Maybe Int
listIndexOf needle haystack =
    let
        search i rest =
            if List.take (List.length needle) rest == needle then
                Just i

            else
                case rest of
                    [] ->
                        Nothing

                    _ :: more ->
                        search (i + 1) more
    in
    search 0 haystack


hexByte : Int -> String
hexByte byte =
    String.fromList [ hexDigit (byte // 16), hexDigit (modBy 16 byte) ]


hexDigit : Int -> Char
hexDigit d =
    String.slice d (d + 1) "0123456789abcdef" |> String.toList |> List.head |> Maybe.withDefault '?'
