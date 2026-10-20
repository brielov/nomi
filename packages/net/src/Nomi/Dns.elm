module Nomi.Dns exposing
    ( lookup
    , Error(..), errorToString
    )

{-| Names: the addresses a host name goes by, as the system resolves it.

    Nomi.Dns.lookup "example.com" env.net

On `Net.loopback`, the only names are this machine's: `localhost` goes by
`127.0.0.1`, and an address by itself.

@docs lookup


# Errors

@docs Error, errorToString

-}

import Elm.Kernel.NomiDns
import Nomi.Net exposing (Net)
import Task exposing (Task)


{-| The addresses the name goes by, IPv4 and IPv6, in the order the system
gives them. The network must allow the name.
-}
lookup : String -> Net -> Task Error (List String)
lookup =
    Elm.Kernel.NomiDns.lookup


{-| Why a name has no addresses: the network doesn't allow it, no address
goes by it, or something else, said.
-}
type Error
    = NotAllowed String
    | NotFound String
    | Other String


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        NotAllowed host ->
            host ++ ": not a host the network allows"

        NotFound host ->
            host ++ ": no address goes by this name"

        Other message ->
            message
