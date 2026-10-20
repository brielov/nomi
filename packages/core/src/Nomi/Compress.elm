module Nomi.Compress exposing (Format(..), compress, decompress, compressing, decompressing)

{-| Compression: gzip and deflate, as HTTP and files use them, brotli and
zstd. Small values are compressed whole, by pure functions; streams are
compressed as they are read, by readers that a task makes, as each holds
the state of its compression.

@docs Format, compress, decompress, compressing, decompressing

-}

import Bytes exposing (Bytes)
import Elm.Kernel.NomiCompress
import Nomi.Bytes
import Nomi.Stream as Stream
import Task exposing (Task)


{-| A compression format.
-}
type Format
    = Gzip
    | Deflate
    | Brotli
    | Zstd


name : Format -> String
name format =
    case format of
        Gzip ->
            "gzip"

        Deflate ->
            "deflate"

        Brotli ->
            "brotli"

        Zstd ->
            "zstd"


{-| The bytes, compressed.
-}
compress : Format -> Bytes -> Bytes
compress format =
    Elm.Kernel.NomiCompress.compress (name format)


{-| The bytes, decompressed, or `Nothing` when they aren't the format's or
would unpack to more than 64 MiB, as a few bytes made to can; a larger
value is read through `decompressing`, and `Nomi.Stream.limit`.
-}
decompress : Format -> Bytes -> Maybe Bytes
decompress format =
    Elm.Kernel.NomiCompress.decompress (name format)


{-| A reader of what the reader gives, compressed as it is read.

    Nomi.Fs.reader "access.log" env.fs
        |> Task.andThen (Nomi.Compress.compressing Nomi.Compress.Gzip)

-}
compressing : Format -> Stream.Reader -> Task x Stream.Reader
compressing =
    transform True


{-| A reader of what the reader gives, decompressed as it is read. Input
that isn't the format's fails a read with `Nomi.Stream.Invalid`.
-}
decompressing : Format -> Stream.Reader -> Task x Stream.Reader
decompressing =
    transform False


transform : Bool -> Format -> Stream.Reader -> Task x Stream.Reader
transform compressing_ format from =
    Elm.Kernel.NomiCompress.codec compressing_ (name format)
        |> Task.andThen (\codec -> Stream.reader (next codec from))


{-| The codec's next output: from what the reader gives, as much as it
takes to give some, and the rest once the reader ends.
-}
next : Codec -> Stream.Reader -> Task Stream.Error (Maybe Bytes)
next codec from =
    Stream.read from
        |> Task.andThen
            (\chunk ->
                case chunk of
                    Just input ->
                        Elm.Kernel.NomiCompress.push input codec
                            |> Task.andThen
                                (\output ->
                                    if Nomi.Bytes.length output == 0 then
                                        next codec from

                                    else
                                        Task.succeed (Just output)
                                )

                    Nothing ->
                        Elm.Kernel.NomiCompress.finish codec
            )


type Codec
    = Codec
