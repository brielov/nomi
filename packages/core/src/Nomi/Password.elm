module Nomi.Password exposing (hash, verify)

{-| Passwords, kept as hashes made to be slow to guess: argon2id, by Bun.

@docs hash, verify

-}

import Elm.Kernel.NomiCrypto
import Task exposing (Task)


{-| The password's argon2id hash, with its salt and parameters, in the PHC
form that starts `$argon2id$`, to keep in place of the password.
-}
hash : String -> Task x String
hash =
    Elm.Kernel.NomiCrypto.hashPassword


{-| Whether the password is the one the hash was made of: an argon2 hash,
or a bcrypt one from another system. A hash it can't read is `False`.
-}
verify : String -> String -> Task x Bool
verify =
    Elm.Kernel.NomiCrypto.verifyPassword
