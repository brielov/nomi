module CryptoTest exposing (suite)

import Bytes exposing (Bytes)
import Bytes.Decode as Decode
import Bytes.Encode as Encode
import Expect
import Fuzz exposing (Fuzzer)
import Nomi.Bytes
import Nomi.Compress as Compress exposing (Format(..))
import Nomi.Crypto as Crypto exposing (Algorithm(..))
import Nomi.Password as Password
import Nomi.Random as Random
import Nomi.Ref as Ref
import Nomi.Stream as Stream
import Nomi.Test exposing (effect)
import Process
import Task exposing (Task)
import Test exposing (Test, describe, fuzz, fuzz2, test)


suite : Test
suite =
    describe "nomi/core's crypto, randomness, passwords and compression"
        [ hashes
        , randomness
        , passwords
        , compression
        ]



-- HASHES


hashes : Test
hashes =
    describe "hashes against published vectors"
        [ test "SHA-1, SHA-2, SHA-3 and BLAKE2b of \"abc\" (FIPS 180, FIPS 202, RFC 7693)" <|
            \_ ->
                [ Sha1, Sha256, Sha384, Sha512, Sha3_256, Sha3_512, Blake2b512 ]
                    |> List.map (\algorithm -> Nomi.Bytes.toHex (Crypto.hash algorithm (Nomi.Bytes.fromString "abc")))
                    |> Expect.equal
                        [ "a9993e364706816aba3e25717850c26c9cd0d89d"
                        , "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                        , "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
                        , "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
                        , "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
                        , "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0"
                        , "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
                        ]
        , test "SHA-256 of nothing" <|
            \_ ->
                Nomi.Bytes.toHex (Crypto.hash Sha256 (Nomi.Bytes.fromString ""))
                    |> Expect.equal "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        , test "HMAC-SHA-1 and HMAC-SHA-256 (RFC 2202 and RFC 4231, case 2)" <|
            \_ ->
                [ Sha1, Sha256 ]
                    |> List.map (\algorithm -> Nomi.Bytes.toHex (Crypto.hmac algorithm (Nomi.Bytes.fromString "Jefe") (Nomi.Bytes.fromString "what do ya want for nothing?")))
                    |> Expect.equal
                        [ "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
                        , "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
                        ]
        , test "HMAC-SHA-512 with a 20-byte key (RFC 4231, case 1)" <|
            \_ ->
                Crypto.hmac Sha512 (fromList (List.repeat 20 0x0B)) (Nomi.Bytes.fromString "Hi There")
                    |> Nomi.Bytes.toHex
                    |> Expect.equal "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
        , fuzz2 bytes bytes "equalConstantTime is whether the bytes are the same" <|
            \a b ->
                ( Crypto.equalConstantTime a b, Crypto.equalConstantTime a (fromList (toList a)) )
                    |> Expect.equal ( toList a == toList b, True )
        , effect "hashReader hashes a stream as hash hashes it whole" <|
            \_ ->
                chunks [ "a", "b", "c" ]
                    |> Task.andThen (Crypto.hashReader Sha256)
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Nomi.Bytes.toHex >> Expect.equal "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        ]



-- RANDOMNESS


randomness : Test
randomness =
    describe "randomness"
        [ effect "a test's random values are its own, the same every run" <|
            \_ ->
                Task.map2 Tuple.pair (Random.bytes 8) (Random.bytes 8)
                    |> Task.map (\( a, b ) -> ( Nomi.Bytes.toHex a, Nomi.Bytes.toHex b ))
                    |> Task.map
                        (\( a, b ) ->
                            Expect.all
                                [ \_ -> Expect.notEqual a b
                                , \_ -> Expect.equal a (Nomi.Bytes.toHex (fromList seeded))
                                ]
                                ()
                        )
        , effect "another test's are others" <|
            \_ -> Random.bytes 8 |> Task.map (Nomi.Bytes.toHex >> Expect.notEqual (Nomi.Bytes.toHex (fromList seeded)))
        , effect "an int is within its bounds, either way round" <|
            \_ ->
                Task.sequence (List.repeat 200 (Random.int 10 1))
                    |> Task.map (\ints -> ( List.minimum ints, List.maximum ints ))
                    |> Task.map (Expect.equal ( Just 1, Just 10 ))
        , effect "a version 4 UUID has its version and variant" <|
            \_ ->
                Random.uuid
                    |> Task.map (\id -> ( String.length id, String.slice 14 15 id, List.member (String.slice 19 20 id) [ "8", "9", "a", "b" ] ))
                    |> Task.map (Expect.equal ( 36, "4", True ))
        , effect "a version 7 UUID starts with the clock's time, so later ones sort after" <|
            \_ ->
                Random.uuidV7
                    |> Task.andThen (\first -> Process.sleep 1000 |> Task.andThen (\_ -> Random.uuidV7) |> Task.map (Tuple.pair first))
                    |> Task.map (\( first, second ) -> ( String.left 15 first, String.left 15 second, first < second ))
                    |> Task.map (Expect.equal ( "00000000-0000-7", "00000000-03e8-7", True ))
        ]


{-| The first 8 bytes "a test's random values are its own" gets, which
its description seeds.
-}
seeded : List Int
seeded =
    [ 0xDE, 0x28, 0xC8, 0x89, 0x12, 0xE7, 0x78, 0x15 ]



-- PASSWORDS


passwords : Test
passwords =
    describe "passwords"
        [ effect "an argon2id hash verifies its password, and no other" <|
            \_ ->
                Password.hash "correct horse"
                    |> Task.andThen
                        (\hashed ->
                            Task.map2 (\right wrong -> ( String.left 10 hashed, right, wrong ))
                                (Password.verify "correct horse" hashed)
                                (Password.verify "wrong horse" hashed)
                        )
                    |> Task.map (Expect.equal ( "$argon2id$", True, False ))
        , effect "a bcrypt hash from elsewhere verifies, and a hash that is none doesn't" <|
            \_ ->
                Task.map3 (\right wrong none -> ( right, wrong, none ))
                    (Password.verify "correct horse" "$2b$04$UO4fwS9O/mHkOqousoUww.zK5UdX03H43.n0I0JvXel3a2ipE1z5K")
                    (Password.verify "wrong horse" "$2b$04$UO4fwS9O/mHkOqousoUww.zK5UdX03H43.n0I0JvXel3a2ipE1z5K")
                    (Password.verify "correct horse" "not a hash")
                    |> Task.map (Expect.equal ( True, False, False ))
        ]



-- COMPRESSION


compression : Test
compression =
    describe "compression"
        [ fuzz2 format bytes "whole values round-trip" <|
            \f b -> Compress.decompress f (Compress.compress f b) |> Maybe.map toList |> Expect.equal (Just (toList b))
        , test "corrupt input decompresses to nothing" <|
            \_ ->
                [ Gzip, Deflate, Brotli, Zstd ]
                    |> List.map (\f -> Compress.decompress f (fromList [ 1, 2, 3, 4, 5 ]))
                    |> List.map (Maybe.map toList)
                    |> Expect.equal [ Nothing, Nothing, Nothing, Nothing ]
        , effect "streams round-trip, and a streamed compression decompresses whole" <|
            \_ ->
                [ Gzip, Deflate, Brotli, Zstd ]
                    |> List.map
                        (\f ->
                            chunks (List.repeat 50 "a line of text that repeats\n")
                                |> Task.andThen (Compress.compressing f)
                                |> Task.andThen (Stream.readAll { limit = 1000000 })
                                |> Task.andThen
                                    (\packed ->
                                        Stream.fromBytes packed
                                            |> Task.andThen (Compress.decompressing f)
                                            |> Task.andThen (Stream.readAll { limit = 1000000 })
                                            |> Task.map (\unpacked -> ( Nomi.Bytes.toString unpacked, Maybe.andThen Nomi.Bytes.toString (Compress.decompress f packed) ))
                                    )
                        )
                    |> Task.sequence
                    |> Task.mapError Stream.errorToString
                    |> Task.map (List.map (\( streamed, whole ) -> streamed == whole && Maybe.map String.length streamed == Just 1400))
                    |> Task.map (Expect.equal [ True, True, True, True ])
        , effect "a corrupt stream fails a read with Invalid" <|
            \_ ->
                Stream.fromBytes (fromList [ 1, 2, 3, 4, 5 ])
                    |> Task.andThen (Compress.decompressing Gzip)
                    |> Task.andThen (Stream.readAll { limit = 1000 })
                    |> Task.map (\_ -> "read")
                    |> Task.onError
                        (\error ->
                            case error of
                                Stream.Invalid _ ->
                                    Task.succeed "Invalid"

                                other ->
                                    Task.succeed (Debug.toString other)
                        )
                    |> Task.map (Expect.equal "Invalid")
        ]



-- HELPERS


format : Fuzzer Format
format =
    Fuzz.oneOfValues [ Gzip, Deflate, Brotli, Zstd ]


bytes : Fuzzer Bytes
bytes =
    Fuzz.listOfLengthBetween 0 200 (Fuzz.intRange 0 255) |> Fuzz.map fromList


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


chunks : List String -> Task x Stream.Reader
chunks given =
    Ref.make given
        |> Task.andThen
            (\left ->
                Stream.reader
                    (Ref.modify (\rest -> ( List.head rest, List.drop 1 rest )) left
                        |> Task.map (Maybe.map Nomi.Bytes.fromString)
                    )
            )
