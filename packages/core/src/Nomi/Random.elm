module Nomi.Random exposing (bytes, int, uuid, uuidV7)

{-| Random values from the system's secure source, for keys, tokens and
ids. Under `nomi test` each test gets values of its own, the same on
every run, from a generator its description seeds.

@docs bytes, int, uuid, uuidV7

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiCrypto
import Task exposing (Task)


{-| This many random bytes.

    Nomi.Random.bytes 32 |> Task.map Nomi.Bytes.toBase64Url

-}
bytes : Int -> Task x Bytes
bytes =
    Elm.Kernel.NomiCrypto.random


{-| An integer from the first to the second, each as likely.
-}
int : Int -> Int -> Task x Int
int =
    Elm.Kernel.NomiCrypto.int


{-| A random UUID, version 4: `"f47ac10b-58cc-4372-a567-0e02b2c3d479"`.
-}
uuid : Task x String
uuid =
    Elm.Kernel.NomiCrypto.uuid


{-| A UUID of version 7, which starts with the clock's time, so ids made
later sort after those made before, as a database's index likes.
-}
uuidV7 : Task x String
uuidV7 =
    Elm.Kernel.NomiCrypto.uuidV7
