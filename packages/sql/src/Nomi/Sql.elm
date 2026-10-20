module Nomi.Sql exposing
    ( Db, connect, open, memory, close
    , Config, config, configFromEnv, Tls(..), withTls, withPoolSize
    , query, execute, transaction
    , Listener, listen, notification
    , Value(..)
    , Decoder, field, int, int64, float, string, bytes, bool, nullable, value, columns
    , succeed, fail, map, map2, map3, map4, map5, andThen, oneOf
    , Error(..), Constraint(..), errorToString
    )

{-| SQL databases through `Bun.sql`: Postgres and MySQL, over the network,
and SQLite, on a file or in memory.

    Nomi.Sql.open "todos.db" env.fs
        |> Task.andThen
            (\db ->
                Nomi.Sql.execute "insert into todos (title) values (?)" [ Nomi.Sql.Text "milk" ] db
                    |> Task.andThen (\_ -> Nomi.Sql.query "select id, title from todos" [] todo db)
            )

    todo : Nomi.Sql.Decoder Todo
    todo =
        Nomi.Sql.map2 Todo
            (Nomi.Sql.field "id" Nomi.Sql.int)
            (Nomi.Sql.field "title" Nomi.Sql.string)

A statement's parameters are its placeholders, given in order, so a value
is never spliced into its text: `?` on SQLite and MySQL, and `$1`, `$2`
and on on Postgres, which can name one twice. A Postgres or MySQL
statement whose fiber is interrupted is cancelled on its server, from a
connection outside the pool, which a pool every fiber holds can't keep
waiting. A SQLite statement runs on the event loop: while it runs, every
fiber waits, a server's requests included, and a fiber interrupted
meanwhile ends once it returns. Keep statements short, and page a large
result with `limit` and `offset`, as each comes back whole.

This package is experimental until a release has run it in production.

@docs Db, connect, open, memory, close


# Servers

@docs Config, config, configFromEnv, Tls, withTls, withPoolSize


# Statements

@docs query, execute, transaction


# Notifications

@docs Listener, listen, notification


# Values

@docs Value


# Rows

@docs Decoder, field, int, int64, float, string, bytes, bool, nullable, value, columns
@docs succeed, fail, map, map2, map3, map4, map5, andThen, oneOf


# Errors

@docs Error, Constraint, errorToString

-}

import Bytes exposing (Bytes)
import Dict exposing (Dict)
import Elm.Kernel.NomiSql
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Net exposing (Net)
import Task exposing (Task)



-- DATABASES


{-| A database, open: the fiber that opened it owns it, and it closes once
that fiber ends, or on `close`. Fibers share it, each statement running on
a connection of its pool, or waiting for one to be free; a `transaction`
holds one until it ends. SQLite's pool is its one connection, so there
the others' statements wait for a transaction to end.
-}
type Db
    = Db


{-| A pool of connections to the Postgres or MySQL server the config
names, through the network, which must allow its host. The first
connection is made at once, so a server that can't be reached, or
refuses the credentials, fails here; the others as fibers need them, up
to the pool's size.

    Nomi.Sql.configFromEnv env.vars
        |> Result.map (\config -> Nomi.Sql.connect config env.net)

-}
connect : Config -> Net -> Task Error Db
connect (Config c) net =
    Elm.Kernel.NomiSql.connect c.url (tlsMode c.tls) c.poolSize net


{-| The SQLite database in the file at the path, created if it isn't there
and the file system may write, read-only if it may not (`Fs.readOnly`).
Its foreign keys are enforced, as SQLite doesn't by default. A path
outside the capability's root fails as `Fs` does, and a file system with
no disk behind it, as `Fs.memory`, with `NotOnDisk`.
-}
open : String -> Fs -> Task Error Db
open =
    Elm.Kernel.NomiSql.open


{-| A SQLite database in memory, empty, its foreign keys enforced: the
real thing, so tests use it.
-}
memory : Task x Db
memory =
    Elm.Kernel.NomiSql.memory


{-| Closes the database: what uses it afterwards fails with `Closed`.
-}
close : Db -> Task x ()
close =
    Elm.Kernel.NomiSql.close



-- SERVERS


{-| Where a Postgres or MySQL server is, and how to reach it: its URL, as
`postgres://user:password@host:5432/database` or
`mysql://user:password@host:3306/database`, how the connection is
secured, and the most connections the pool holds, 10 unless given.
-}
type Config
    = Config { url : String, tls : Tls, poolSize : Int }


{-| How a connection is secured, as libpq's `sslmode` names it: not at
all (`disable`); with TLS where the server offers it, its certificate
unchecked (`prefer`, the default); always with TLS, its certificate
unchecked (`require`); or always with TLS, its certificate checked for the
host's name (`verify-full`), which a server reached over a network
others share should be. MySQL sends a password only over TLS unless the
client fetches its key first, which a server in the middle could forge,
so nomi never does: to MySQL, `disable` fails to sign in.
-}
type Tls
    = NoTls
    | PreferTls
    | RequireTls
    | VerifiedTls


{-| The config of a URL, its scheme `postgres`, `postgresql` or `mysql`;
its `sslmode` parameter, if it has one, says how it is secured. Other
parameters are Bun's. A URL that isn't one fails, without repeating it,
as it may hold a password.
-}
config : String -> Result String Config
config url =
    let
        ( base, parameters ) =
            case String.indexes "?" url of
                at :: _ ->
                    ( String.left at url, String.split "&" (String.dropLeft (at + 1) url) )

                [] ->
                    ( url, [] )

        mode =
            List.filterMap
                (\parameter_ ->
                    case String.split "=" parameter_ of
                        [ "sslmode", value_ ] ->
                            Just value_

                        _ ->
                            Nothing
                )
                parameters

        others =
            List.filter (\parameter_ -> not (String.startsWith "sslmode=" parameter_)) parameters

        withOthers =
            if List.isEmpty others then
                base

            else
                base ++ "?" ++ String.join "&" others
    in
    if not (List.any (\scheme -> String.startsWith (scheme ++ "://") url) [ "postgres", "postgresql", "mysql" ]) then
        Err "not a postgres://, postgresql:// or mysql:// URL"

    else
        case mode of
            [] ->
                Ok (Config { url = withOthers, tls = PreferTls, poolSize = 10 })

            [ name ] ->
                case tlsNamed name of
                    Just tls ->
                        Ok (Config { url = withOthers, tls = tls, poolSize = 10 })

                    Nothing ->
                        Err ("sslmode=" ++ name ++ ": not disable, prefer, require or verify-full")

            _ ->
                Err "sslmode is given more than once"


tlsNamed : String -> Maybe Tls
tlsNamed name =
    case name of
        "disable" ->
            Just NoTls

        "prefer" ->
            Just PreferTls

        "require" ->
            Just RequireTls

        "verify-full" ->
            Just VerifiedTls

        _ ->
            Nothing


tlsMode : Tls -> String
tlsMode tls =
    case tls of
        NoTls ->
            "disable"

        PreferTls ->
            "prefer"

        RequireTls ->
            "require"

        VerifiedTls ->
            "verify-full"


{-| The config of the environment's `DATABASE_URL`.

    Nomi.Sql.configFromEnv env.vars

-}
configFromEnv : Dict String String -> Result String Config
configFromEnv vars =
    Dict.get "DATABASE_URL" vars
        |> Result.fromMaybe "DATABASE_URL is not set"
        |> Result.andThen config


{-| The config, its connections secured so.
-}
withTls : Tls -> Config -> Config
withTls tls (Config c) =
    Config { c | tls = tls }


{-| The config, its pool holding at most this many connections, one at
least.
-}
withPoolSize : Int -> Config -> Config
withPoolSize size (Config c) =
    Config { c | poolSize = max 1 size }



-- STATEMENTS


{-| The rows the statement gives, each read by the decoder, failing with
`BadRow` at the first it can't read.

    Nomi.Sql.query "select title from todos where done = ?" [ Nomi.Sql.Bool False ] (Nomi.Sql.field "title" Nomi.Sql.string) db

-}
query : String -> List Value -> Decoder row -> Db -> Task Error (List row)
query sql parameters decoder db =
    Elm.Kernel.NomiSql.query sql (List.map parameter parameters) db
        |> Task.andThen (decodeRows decoder 1 [])


decodeRows : Decoder row -> Int -> List row -> List (List ( String, Value )) -> Task Error (List row)
decodeRows (Decoder decode) index decoded rows =
    case rows of
        [] ->
            Task.succeed (List.reverse decoded)

        row :: rest ->
            case decode (Row row) of
                Ok one ->
                    decodeRows (Decoder decode) (index + 1) (one :: decoded) rest

                Err why ->
                    Task.fail (BadRow ("row " ++ String.fromInt index ++ ": " ++ why))


{-| Runs the statement, or a script of them when it takes no parameters,
for what it changes: how many rows the last statement inserted, updated
or deleted.
-}
execute : String -> List Value -> Db -> Task Error { changes : Int }
execute sql parameters =
    Elm.Kernel.NomiSql.execute sql (List.map parameter parameters)


{-| Runs the task on the database in a transaction, which commits once it
succeeds and rolls back once it fails, with a database's error or the
program's own, or is interrupted. The task is given the transaction, to
run its statements on, and holds a connection of the pool while it runs,
on SQLite its only one, so there other fibers' statements wait for it. A
transaction within it is a savepoint, which rolls back alone. The function
turns the database's errors in beginning and committing into the
program's.

    Nomi.Sql.transaction DatabaseError
        (\tx ->
            Nomi.Sql.execute "update accounts set balance = balance - 10 where id = ?" [ Nomi.Sql.Int 1 ] tx
                |> Task.andThen (\_ -> Nomi.Sql.execute "update accounts set balance = balance + 10 where id = ?" [ Nomi.Sql.Int 2 ] tx)
                |> Task.mapError DatabaseError
        )
        db

-}
transaction : (Error -> e) -> (Db -> Task e a) -> Db -> Task e a
transaction =
    Elm.Kernel.NomiSql.transaction



-- NOTIFICATIONS


{-| A listener of a Postgres channel, on a connection of its own: the
fiber that made it owns it, and it stops listening once that fiber ends.
-}
type Listener
    = Listener


{-| Listens on the Postgres channel, for what `NOTIFY` or `pg_notify`
sends there. Bun's listener reconnects when its connection is lost, and
what was sent meanwhile is lost with it, so a listener whose connection
was lost fails with `Unreachable` instead, for the program to listen anew
and catch up from its tables. It holds the 1,000 notifications its fibers
haven't taken, and fails past them, with code `54000`, rather than grow.
On MySQL and SQLite, which have no channels, it fails with code `0A000`.
-}
listen : String -> Db -> Task Error Listener
listen =
    Elm.Kernel.NomiSql.listen


{-| The next notification's payload, once one comes, or `Nothing` once the
listener has stopped.

    Nomi.Sql.notification listener
        |> Task.andThen
            (\payload ->
                case payload of
                    Just todo ->
                        refresh todo

                    Nothing ->
                        Task.succeed ()
            )

-}
notification : Listener -> Task Error (Maybe String)
notification =
    Elm.Kernel.NomiSql.notification



-- VALUES


{-| A parameter, and what a column holds. An `Int` past ±(2^53 − 1), the
integers a JavaScript number holds exactly, is refused as a parameter,
as it is already rounded: `Int64` passes any 64-bit integer as its
decimal text, and a column holding one past that range reads as one.
SQLite has no booleans, storing `Bool` as 1 or 0, and MySQL's are 1 or 0
too. A `numeric` or `decimal` reads as `Text`, exactly; a date or time as
`Text` in ISO 8601, to the millisecond; and JSON as its text.
-}
type Value
    = Null
    | Int Int
    | Int64 String
    | Float Float
    | Text String
    | Blob Bytes
    | Bool Bool



{-| A value as the kernel passes it to Bun.
-}
type Parameter
    = Parameter


parameter : Value -> Parameter
parameter held =
    case held of
        Null ->
            Elm.Kernel.NomiSql.null

        Int n ->
            Elm.Kernel.NomiSql.int n

        Int64 text ->
            Elm.Kernel.NomiSql.int64 text

        Float f ->
            Elm.Kernel.NomiSql.float f

        Text text ->
            Elm.Kernel.NomiSql.text text

        Blob blob ->
            Elm.Kernel.NomiSql.blob blob

        Bool b ->
            Elm.Kernel.NomiSql.bool b



-- ROWS


{-| How to read a row, or one of its columns, into a value, as
`Json.Decode` reads JSON: `field` reads a column of a row, by name.
-}
type Decoder a
    = Decoder (Cell -> Result String a)


type Cell
    = Row (List ( String, Value ))
    | Column String Value


{-| The column of the name, read by the decoder.
-}
field : String -> Decoder a -> Decoder a
field name (Decoder decode) =
    Decoder
        (\cell ->
            case cell of
                Row row ->
                    case List.filter (\( key, _ ) -> key == name) row of
                        ( _, found ) :: _ ->
                            decode (Column name found)

                        [] ->
                            Err ("no column " ++ quoted name)

                Column outer _ ->
                    Err ("column " ++ quoted outer ++ " is a value, which has no column " ++ quoted name)
        )


{-| An integer within ±(2^53 − 1). One past that fails rather than round:
read it with `int64`.
-}
int : Decoder Int
int =
    column "an integer"
        (\cell ->
            case cell of
                Int n ->
                    Just (Ok n)

                Int64 text ->
                    Just (Err (text ++ " is past ±(2^53 − 1), the integers an Int holds exactly: read it with int64"))

                _ ->
                    Nothing
        )


{-| Any integer, exactly, as its decimal text.
-}
int64 : Decoder String
int64 =
    column "an integer"
        (\cell ->
            case cell of
                Int n ->
                    Just (Ok (String.fromInt n))

                Int64 text ->
                    Just (Ok text)

                _ ->
                    Nothing
        )


{-| A number, an integer included.
-}
float : Decoder Float
float =
    column "a number"
        (\cell ->
            case cell of
                Float f ->
                    Just (Ok f)

                Int n ->
                    Just (Ok (toFloat n))

                _ ->
                    Nothing
        )


{-| Text.
-}
string : Decoder String
string =
    column "text"
        (\cell ->
            case cell of
                Text text ->
                    Just (Ok text)

                _ ->
                    Nothing
        )


{-| A blob's bytes.
-}
bytes : Decoder Bytes
bytes =
    column "a blob"
        (\cell ->
            case cell of
                Blob blob ->
                    Just (Ok blob)

                _ ->
                    Nothing
        )


{-| A boolean, or 1 or 0, as SQLite stores one.
-}
bool : Decoder Bool
bool =
    column "a boolean"
        (\cell ->
            case cell of
                Bool b ->
                    Just (Ok b)

                Int 1 ->
                    Just (Ok True)

                Int 0 ->
                    Just (Ok False)

                _ ->
                    Nothing
        )


{-| `Nothing` for `NULL`, and otherwise what the decoder reads.
-}
nullable : Decoder a -> Decoder (Maybe a)
nullable (Decoder decode) =
    Decoder
        (\cell ->
            case cell of
                Column _ Null ->
                    Ok Nothing

                _ ->
                    Result.map Just (decode cell)
        )


{-| The column's value as it is.
-}
value : Decoder Value
value =
    column "a value" (\held -> Just (Ok held))


{-| Every column of the row, by name, in the order the statement gives
them, as a program that doesn't know them reads a row.
-}
columns : Decoder (List ( String, Value ))
columns =
    Decoder
        (\cell ->
            case cell of
                Row row ->
                    Ok row

                Column name _ ->
                    Err ("column " ++ quoted name ++ " is a value, which has no columns")
        )


column : String -> (Value -> Maybe (Result String a)) -> Decoder a
column expected read =
    Decoder
        (\cell ->
            case cell of
                Column name held ->
                    case read held of
                        Just result ->
                            Result.mapError (\why -> "column " ++ quoted name ++ ": " ++ why) result

                        Nothing ->
                            Err ("column " ++ quoted name ++ ": expected " ++ expected ++ ", got " ++ describe held)

                Row _ ->
                    Err ("expected " ++ expected ++ ", got a row: read a column of it with field")
        )


describe : Value -> String
describe held =
    case held of
        Null ->
            "NULL"

        Int n ->
            "the integer " ++ String.fromInt n

        Int64 text ->
            "the integer " ++ text

        Float f ->
            "the number " ++ String.fromFloat f

        Text text ->
            "the text " ++ quoted text

        Blob blob ->
            "a blob of " ++ String.fromInt (Bytes.width blob) ++ " bytes"

        Bool b ->
            if b then
                "True"

            else
                "False"


quoted : String -> String
quoted text =
    "\"" ++ text ++ "\""


{-| Reads nothing, giving the value.
-}
succeed : a -> Decoder a
succeed a =
    Decoder (\_ -> Ok a)


{-| Fails, saying why.
-}
fail : String -> Decoder a
fail why =
    Decoder (\_ -> Err why)


{-| -}
map : (a -> b) -> Decoder a -> Decoder b
map f (Decoder a) =
    Decoder (\cell -> Result.map f (a cell))


{-| -}
map2 : (a -> b -> c) -> Decoder a -> Decoder b -> Decoder c
map2 f (Decoder a) (Decoder b) =
    Decoder (\cell -> Result.map2 f (a cell) (b cell))


{-| -}
map3 : (a -> b -> c -> d) -> Decoder a -> Decoder b -> Decoder c -> Decoder d
map3 f (Decoder a) (Decoder b) (Decoder c) =
    Decoder (\cell -> Result.map3 f (a cell) (b cell) (c cell))


{-| -}
map4 : (a -> b -> c -> d -> e) -> Decoder a -> Decoder b -> Decoder c -> Decoder d -> Decoder e
map4 f (Decoder a) (Decoder b) (Decoder c) (Decoder d) =
    Decoder (\cell -> Result.map4 f (a cell) (b cell) (c cell) (d cell))


{-| -}
map5 : (a -> b -> c -> d -> e -> f) -> Decoder a -> Decoder b -> Decoder c -> Decoder d -> Decoder e -> Decoder f
map5 f (Decoder a) (Decoder b) (Decoder c) (Decoder d) (Decoder e) =
    Decoder (\cell -> Result.map5 f (a cell) (b cell) (c cell) (d cell) (e cell))


{-| A decoder chosen by what another read.
-}
andThen : (a -> Decoder b) -> Decoder a -> Decoder b
andThen f (Decoder a) =
    Decoder
        (\cell ->
            a cell
                |> Result.andThen
                    (\read ->
                        let
                            (Decoder b) =
                                f read
                        in
                        b cell
                    )
        )


{-| The first decoder that reads the cell, failing with the last's reason
when none does.
-}
oneOf : List (Decoder a) -> Decoder a
oneOf decoders =
    Decoder (\cell -> firstOf decoders cell "no decoder to try")


firstOf : List (Decoder a) -> Cell -> String -> Result String a
firstOf decoders cell why =
    case decoders of
        [] ->
            Err why

        (Decoder decode) :: rest ->
            case decode cell of
                Ok a ->
                    Ok a

                Err next ->
                    firstOf rest cell next



-- ERRORS


{-| Why a statement failed: the file couldn't be opened, as `Fs` says; the
file system has no disk to hold one; the network doesn't allow the
server's host; the server couldn't be reached, or its connection was
lost; a constraint refused the change; the database refused the statement
otherwise, with its code (Postgres's and MySQL's SQLSTATE, as `42P01`, or
SQLite's, as `SQLITE_ERROR`) and message; a parameter, counted from 1,
can't be passed; a row couldn't be read; or the database, or the
transaction a `Db` stood for, is closed.
-}
type Error
    = FileError Fs.Error
    | NotOnDisk String
    | NotAllowed String
    | Unreachable String
    | Violated Constraint String
    | Failed { code : String, message : String }
    | BadParameter Int String
    | BadRow String
    | Closed


{-| The kind of constraint a change broke: a unique key, the primary key
included; a foreign key; a `NOT NULL` column; or a `CHECK`.
-}
type Constraint
    = Unique
    | ForeignKey
    | NotNull
    | Check


{-| The error in a sentence, for a person.
-}
errorToString : Error -> String
errorToString error =
    case error of
        FileError fsError ->
            Fs.errorToString fsError

        NotOnDisk path ->
            path ++ ": the file system has no disk to hold a database"

        NotAllowed host ->
            host ++ ": not a host the network allows"

        Unreachable why ->
            why

        Violated _ message ->
            message

        Failed { code, message } ->
            code ++ ": " ++ message

        BadParameter index why ->
            "parameter " ++ String.fromInt index ++ ": " ++ why

        BadRow why ->
            why

        Closed ->
            "the database is closed"
