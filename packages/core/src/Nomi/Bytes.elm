module Nomi.Bytes exposing
    ( length, slice, append, concat
    , equal, indexOf
    , fromString, toString
    , toBase64, toBase64Url, fromBase64, toHex, fromHex
    )

{-| What elm/bytes lacks for `Bytes` as data: cutting, joining, comparing
and searching them, UTF-8 text that refuses what isn't UTF-8, and base64
and hex. Each function is pure, and the `Bytes` are elm/bytes' own, so
`Bytes.Encode` and `Bytes.Decode` work on what these give and take.

`==` can't tell `Bytes` apart: to it, any two are equal. Compare them
with `equal`.


# Measuring and cutting

@docs length, slice, append, concat


# Comparing and searching

@docs equal, indexOf


# Text

@docs fromString, toString


# Base64 and hex

@docs toBase64, toBase64Url, fromBase64, toHex, fromHex

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiBytes


{-| How many bytes there are, as `Bytes.width` says.
-}
length : Bytes -> Int
length =
    Elm.Kernel.NomiBytes.length


{-| The bytes from the first index up to the second, which is left out,
as `String.slice` takes characters: an index counts from the end when it
is negative, and stops at either end.

    slice 1 (-1) (fromString "[abc]") -- the bytes of "abc"

-}
slice : Int -> Int -> Bytes -> Bytes
slice =
    Elm.Kernel.NomiBytes.slice


{-| The first bytes, then the second.
-}
append : Bytes -> Bytes -> Bytes
append =
    Elm.Kernel.NomiBytes.append


{-| Each of the bytes, in order.
-}
concat : List Bytes -> Bytes
concat =
    Elm.Kernel.NomiBytes.concat


{-| Whether the two hold the same bytes.
-}
equal : Bytes -> Bytes -> Bool
equal =
    Elm.Kernel.NomiBytes.equal


{-| Where the first bytes first appear in the second, counting from 0, or
`Nothing` when they don't. Empty bytes appear at 0.

    indexOf (fromString "\r\n") (fromString "GET / HTTP/1.1\r\n") -- Just 14

-}
indexOf : Bytes -> Bytes -> Maybe Int
indexOf =
    Elm.Kernel.NomiBytes.indexOf


{-| The text in UTF-8, as `Bytes.Encode.string` writes it. A string may
hold half a surrogate pair, which UTF-8 has no bytes for: each half
becomes U+FFFD, the replacement character, where `Bytes.Encode.string`
writes bytes that aren't UTF-8.
-}
fromString : String -> Bytes
fromString =
    Elm.Kernel.NomiBytes.fromString


{-| The text the bytes hold in UTF-8, or `Nothing` when they aren't
UTF-8: a byte no character starts with, a character cut short, one
written in more bytes than it takes, half a surrogate pair, or a
character past U+10FFFF. Nothing is replaced, where `Bytes.Decode.string`
reads any bytes as some text. A byte order mark at the start is kept, as
U+FEFF.
-}
toString : Bytes -> Maybe String
toString =
    Elm.Kernel.NomiBytes.toString


{-| The bytes in base64, with RFC 4648's standard alphabet and padding:
`"+/8B"`, or `"Zm8="` for the bytes of `"fo"`.
-}
toBase64 : Bytes -> String
toBase64 =
    Elm.Kernel.NomiBytes.toBase64


{-| The bytes in base64 for URLs and file names: `-` and `_` in place of
`+` and `/`, and no padding, as JSON Web Tokens write it.
-}
toBase64Url : Bytes -> String
toBase64Url =
    Elm.Kernel.NomiBytes.toBase64Url


{-| The bytes of base64 in either alphabet, padded or not, as `toBase64`
and `toBase64Url` write it, or `Nothing` for anything else: another
character, whitespace too, padding short of a group of four, or a last
character whose bits past the last byte aren't zero, so the bytes have
one spelling in each alphabet.
-}
fromBase64 : String -> Maybe Bytes
fromBase64 =
    Elm.Kernel.NomiBytes.fromBase64


{-| The bytes in hex, two lowercase digits each: `"fbff01"`.
-}
toHex : Bytes -> String
toHex =
    Elm.Kernel.NomiBytes.toHex


{-| The bytes of hex in either case, or `Nothing` for an odd number of
digits or anything else.
-}
fromHex : String -> Maybe Bytes
fromHex =
    Elm.Kernel.NomiBytes.fromHex
