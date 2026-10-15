module Nomi.Ffi exposing (task, Error(..))

{-| Calls to JavaScript: the async functions an entry's FFI module exports.

@docs task, Error

-}

import Elm.Kernel.NomiFfi
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Task exposing (Task)


{-| Calls the FFI module's export of the name as `f(arg, signal)` and
decodes what its promise fulfils with. Interrupting the task aborts
`signal`. A name written as a literal must be one the module exports,
or the build fails.

    Nomi.Ffi.task "hash" (Encode.string password) Decode.string

-}
task : String -> Encode.Value -> Decoder a -> Task Error a
task name arg decoder =
    Elm.Kernel.NomiFfi.call name arg
        |> Task.mapError Threw
        |> Task.andThen
            (\result ->
                case Decode.decodeValue decoder result of
                    Ok value ->
                        Task.succeed value

                    Err error ->
                        Task.fail (BadResult (Decode.errorToString error))
            )


{-| How a call failed: the function threw or its promise rejected, or what
it gave did not decode.
-}
type Error
    = Threw String
    | BadResult String
