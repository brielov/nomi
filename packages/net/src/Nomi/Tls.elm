module Nomi.Tls exposing
    ( connect, Trust(..)
    , listen, Certificate
    , upgrade, accept
    )

{-| TCP connections over TLS: a connection is a `Nomi.Tcp.Connection`
like any other, its bytes encrypted on the wire.

    Nomi.Tls.connect Nomi.Tls.System { host = "example.com", port_ = 443 } env.net

A client checks the server's certificate before the connection is made:
it must be signed by a certificate the trust holds, and be for the host
connected to, or the connection fails with `Untrusted`.

On `Net.loopback`, nothing is on a wire: a connection over TLS is one in
memory, as any other there, and no certificate is checked.

@docs connect, Trust


# Servers

@docs listen, Certificate


# Upgrading

A protocol that starts in plain text and turns to TLS, as SMTP's and
Postgres's STARTTLS do, upgrades the connection it has once both sides
agree.

@docs upgrade, accept

-}

import Elm.Kernel.NomiTcp
import Nomi.Net exposing (Net)
import Nomi.Tcp as Tcp exposing (Connection)
import Task exposing (Task)


{-| The certificates a server's must be signed by: the system's, or these,
in PEM, as for a server whose certificate a private authority signed.
-}
type Trust
    = System
    | Certificates String


trustPem : Trust -> String
trustPem trust =
    case trust of
        System ->
            ""

        Certificates pem ->
            pem


{-| A connection over TLS to the host, by name or address, on the port,
once the server's certificate is trusted for the host.
-}
connect : Trust -> { host : String, port_ : Int } -> Net -> Task Tcp.Error Connection
connect trust { host, port_ } =
    Elm.Kernel.NomiTcp.connectTls (trustPem trust) host port_


{-| A server's certificate chain and its private key, in PEM.
-}
type alias Certificate =
    { certificate : String
    , key : String
    }


{-| `Nomi.Tcp.listen` over TLS, with the certificate.
-}
listen : Certificate -> Tcp.Config -> (Connection -> Task Never ()) -> Net -> Task Tcp.Error ()
listen { certificate, key } config =
    Elm.Kernel.NomiTcp.listenTls certificate key config.host config.port_ config.onListen


{-| The client's side of an upgrade: the connection over TLS, the server's
certificate trusted for the host, from which point the connection given
is the one this gives, and reading or writing the one given fails.
-}
upgrade : Trust -> String -> Connection -> Task Tcp.Error Connection
upgrade trust host =
    Elm.Kernel.NomiTcp.upgrade (trustPem trust) host


{-| The server's side of an upgrade, with its certificate.
-}
accept : Certificate -> Connection -> Task Tcp.Error Connection
accept { certificate, key } =
    Elm.Kernel.NomiTcp.accept certificate key
