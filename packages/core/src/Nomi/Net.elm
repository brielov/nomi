module Nomi.Net exposing (Net, onlyHosts, noListen, loopback)

{-| The network, as a capability: a program reaches the hosts, and listens
on the ports, that the `Net` it is handed allows, which is the whole
network for `Nomi.Env`'s `net`.

@docs Net, onlyHosts, noListen, loopback

-}

import Elm.Kernel.NomiNet
import Task exposing (Task)


{-| The capability to connect to hosts and to listen for connections.
-}
type Net
    = Net


{-| The same network, reaching only these hosts, by name or address as a
URL gives them, in any case: `"api.example.com"` or `"127.0.0.1"`. A
host it was narrowed from already must be among them, so narrowing again
only takes hosts away. Each redirect a client follows is checked too.

    Nomi.Net.onlyHosts [ "api.github.com" ] env.net

-}
onlyHosts : List String -> Net -> Net
onlyHosts =
    Elm.Kernel.NomiNet.onlyHosts


{-| The same network, on which a server can't listen
(`PermissionDenied`).
-}
noListen : Net -> Net
noListen =
    Elm.Kernel.NomiNet.noListen


{-| A network of its own, in memory: a server listening on it answers the
clients that send on it, with no socket between them, and nothing else is
there. Under `nomi test` a request on it takes no real time, so a handler
that sleeps or times out is tested through it on the test's clock. Its
responses carry no headers about the connection, as `Date` or
`Content-Length`.
-}
loopback : Task x Net
loopback =
    Elm.Kernel.NomiNet.loopback
