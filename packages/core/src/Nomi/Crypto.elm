module Nomi.Crypto exposing (Algorithm(..), hash, hmac, equalConstantTime, hashReader)

{-| Hashes and message authentication, as pure functions on bytes.

    Nomi.Crypto.hash Nomi.Crypto.Sha256 (Nomi.Bytes.fromString "abc")
        |> Nomi.Bytes.toHex
    -- "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"

@docs Algorithm, hash, hmac, equalConstantTime, hashReader

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiCrypto
import Nomi.Stream as Stream
import Task exposing (Task)


{-| A hash: SHA-1, kept for the protocols that still name it, SHA-2, SHA-3
and BLAKE2b.
-}
type Algorithm
    = Sha1
    | Sha256
    | Sha384
    | Sha512
    | Sha3_256
    | Sha3_512
    | Blake2b512


name : Algorithm -> String
name algorithm =
    case algorithm of
        Sha1 ->
            "sha1"

        Sha256 ->
            "sha256"

        Sha384 ->
            "sha384"

        Sha512 ->
            "sha512"

        Sha3_256 ->
            "sha3-256"

        Sha3_512 ->
            "sha3-512"

        Blake2b512 ->
            "blake2b512"


{-| The bytes' hash.
-}
hash : Algorithm -> Bytes -> Bytes
hash algorithm =
    Elm.Kernel.NomiCrypto.hash (name algorithm)


{-| The HMAC of the message with the key, as a webhook's signature is.

    Nomi.Crypto.hmac Nomi.Crypto.Sha256 secret body

-}
hmac : Algorithm -> Bytes -> Bytes -> Bytes
hmac algorithm =
    Elm.Kernel.NomiCrypto.hmac (name algorithm)


{-| Whether the bytes are the same, taking as long whichever bytes differ,
so comparing a signature someone sent with the right one tells them
nothing of how near it came.
-}
equalConstantTime : Bytes -> Bytes -> Bool
equalConstantTime =
    Elm.Kernel.NomiCrypto.equalConstantTime


{-| The hash of everything the reader gives, read a chunk at a time.
-}
hashReader : Algorithm -> Stream.Reader -> Task Stream.Error Bytes
hashReader algorithm from =
    let
        go hasher =
            Stream.read from
                |> Task.andThen
                    (\next ->
                        case next of
                            Just chunk ->
                                Elm.Kernel.NomiCrypto.update chunk hasher |> Task.andThen (\_ -> go hasher)

                            Nothing ->
                                Elm.Kernel.NomiCrypto.digest hasher
                    )
    in
    Elm.Kernel.NomiCrypto.start (name algorithm) |> Task.andThen go
