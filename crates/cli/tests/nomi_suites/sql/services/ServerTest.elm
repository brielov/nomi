module ServerTest exposing (suite)

{-| What `Nomi.Sql` does on a Postgres or MySQL server alone, under
`cargo xtask services`, which moves this module beside the shared tests
and gives `Database` the server.
-}

import Database exposing (Kind(..))
import Expect
import Nomi.Fiber as Fiber
import Nomi.Net as Net exposing (Net)
import Nomi.Sql as Sql exposing (Db, Value(..))
import Nomi.Test exposing (effect)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Sql on a server"
        [ effect "connect refuses a host the network doesn't allow" <|
            \env ->
                connect identity (Net.onlyHosts [ "db.example.com" ] env.net)
                    |> expectError (Sql.NotAllowed "127.0.0.1")
        , effect "a loopback has no server to reach" <|
            \_ ->
                Net.loopback
                    |> Task.andThen (connect identity)
                    |> expectError (Sql.Unreachable "127.0.0.1: connection refused")
        , effect "a server that isn't there fails at connect" <|
            \env ->
                connectTo (String.replace ("@127.0.0.1:" ++ port_) "@127.0.0.1:1" Database.url) identity env.net
                    |> expectUnreachable
        , effect "credentials the server refuses fail at connect, with its code" <|
            \env ->
                connectTo (String.replace ":nomi@" ":wrong@" Database.url) identity env.net
                    |> Task.map (\_ -> Expect.fail "connected")
                    |> Task.onError
                        (\error ->
                            Task.succeed
                                (case error of
                                    Sql.Failed { code } ->
                                        Expect.equal (if Database.kind == Postgres then "28P01" else "28000") code

                                    _ ->
                                        Expect.fail (Sql.errorToString error)
                                )
                        )
        , effect "TLS is what the server takes: prefer connects either way, and what it lacks fails saying so" <|
            \env ->
                case Database.kind of
                    Mysql ->
                        connect (Sql.withTls Sql.NoTls) env.net
                            |> expectError (Sql.Unreachable "127.0.0.1: MySQL takes a password only over TLS, but the connection has none: connect with sslmode prefer or require")

                    _ ->
                        connect (Sql.withTls Sql.RequireTls) env.net
                            |> expectUnreachable
        , effect "a broken connection leaves the pool, and the next statement gets another" <|
            \env ->
                connect (Sql.withPoolSize 1) env.net
                    |> Task.andThen
                        (\db ->
                            backend db
                                |> Task.andThen (\first -> kill first env.net |> Task.map (\_ -> first))
                                |> Task.andThen
                                    (\first ->
                                        backend db
                                            |> Task.map (\_ -> "the killed connection answered")
                                            |> Task.onError (\error -> Task.succeed (failedKind error))
                                            |> Task.andThen (\broke -> backend db |> Task.map (\next -> ( broke, next /= first )))
                                    )
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal ( "unreachable", True ))
        , effect "a statement whose fiber is interrupted is cancelled on the server, with every pooled connection busy" <|
            \env ->
                connect (Sql.withPoolSize 2) env.net
                    |> Task.andThen
                        (\db ->
                            connect identity env.net
                                |> Task.andThen
                                    (\observer ->
                                        Fiber.fork (Sql.execute sleeping [] db)
                                            |> Task.andThen (\first -> Fiber.fork (Sql.execute sleeping [] db) |> Task.map (Tuple.pair first))
                                            |> Task.andThen
                                                (\( first, second ) ->
                                                    running 2 observer
                                                        |> Task.andThen (\both -> Fiber.interrupt first |> Task.andThen (\_ -> running 1 observer) |> Task.map (Tuple.pair both))
                                                        |> Task.andThen (\( both, one ) -> Fiber.interrupt second |> Task.andThen (\_ -> running 0 observer) |> Task.map (\none -> ( both, one, none )))
                                                )
                                            |> Task.andThen (\seen -> backend db |> Task.map (\_ -> seen))
                                    )
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal ( True, True, True ))
        , effect "LISTEN delivers what NOTIFY sends, in order; MySQL has no channels" <|
            \env ->
                case Database.kind of
                    Postgres ->
                        Database.fresh env.net
                            |> Task.andThen
                                (\db ->
                                    Sql.listen "todos" db
                                        |> Task.andThen
                                            (\listener ->
                                                Sql.execute "select pg_notify('todos', 'one'), pg_notify('todos', 'two')" [] db
                                                    |> Task.andThen (\_ -> Task.map2 Tuple.pair (Sql.notification listener) (Sql.notification listener))
                                            )
                                )
                            |> Task.mapError Sql.errorToString
                            |> Task.map (Expect.equal ( Just "one", Just "two" ))

                    _ ->
                        Database.fresh env.net
                            |> Task.andThen (Sql.listen "todos")
                            |> expectError (Sql.Failed { code = "0A000", message = "LISTEN is Postgres's, and MySQL has no channels" })
        , effect "a lost connection fails its listener, rather than drop what was sent meanwhile" <|
            \env ->
                if Database.kind /= Postgres then
                    Task.succeed Expect.pass

                else
                    Database.fresh env.net
                        |> Task.andThen
                            (\db ->
                                Sql.listen "todos" db
                                    |> Task.andThen
                                        (\listener ->
                                            Sql.query "select pid from pg_stat_activity where query ilike 'listen%'" [] (Sql.field "pid" Sql.int) db
                                                |> Task.andThen (\pids -> Task.sequence (List.map (\pid -> Sql.execute ("select pg_terminate_backend(" ++ String.fromInt pid ++ ")") [] db) pids))
                                                |> Task.andThen (\_ -> Sql.notification listener)
                                        )
                            )
                        |> Task.map (\_ -> "a notification came")
                        |> Task.onError (\error -> Task.succeed (failedKind error))
                        |> Task.map (Expect.equal "unreachable")
        , effect "other fibers' statements run beside a transaction, seeing it once it commits" <|
            \env ->
                Database.fresh env.net
                    |> Task.andThen
                        (\db ->
                            Sql.execute "create table todos (title varchar(20))" [] db
                                |> Task.andThen
                                    (\_ ->
                                        Fiber.fork (Sql.transaction identity (\tx -> Sql.execute "insert into todos values ('milk')" [] tx |> Task.andThen (\_ -> Process.sleep 1000)) db)
                                    )
                                |> Task.andThen (\fiber -> Process.sleep 10 |> Task.andThen (\_ -> count db) |> Task.map (Tuple.pair fiber))
                                |> Task.andThen (\( fiber, during ) -> Fiber.join fiber |> Task.andThen (\_ -> count db) |> Task.map (Tuple.pair during))
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal ( 0, 1 ))
        ]


{-| A statement that runs for 30 seconds unless it is cancelled.
-}
sleeping : String
sleeping =
    if Database.kind == Postgres then
        "select pg_sleep(30)"

    else
        "select sleep(30)"


{-| Whether `sleeping` comes to run on the server as many times as `n`,
which the observer asks every 50 ms, on the server, for 5 seconds.
-}
running : Int -> Db -> Task Sql.Error Bool
running n observer =
    let
        ( asking, pausing ) =
            if Database.kind == Postgres then
                ( "select count(*) as n from pg_stat_activity where query = '" ++ sleeping ++ "' and state = 'active'", "select pg_sleep(0.05)" )

            else
                ( "select count(*) as n from information_schema.processlist where info = '" ++ sleeping ++ "'", "select sleep(0.05)" )

        ask tries =
            Sql.query asking [] (Sql.field "n" Sql.int) observer
                |> Task.andThen
                    (\counts ->
                        if counts == [ n ] then
                            Task.succeed True

                        else if tries == 0 then
                            Task.succeed False

                        else
                            Sql.execute pausing [] observer |> Task.andThen (\_ -> ask (tries - 1))
                    )
    in
    ask 100


{-| The URL's port, `Database.url` being `scheme://user:password@127.0.0.1:port/database`.
-}
port_ : String
port_ =
    case String.split "@127.0.0.1:" Database.url of
        [ _, rest ] ->
            String.split "/" rest |> List.head |> Maybe.withDefault ""

        _ ->
            ""


connect : (Sql.Config -> Sql.Config) -> Net -> Task Sql.Error Db
connect =
    connectTo Database.url


connectTo : String -> (Sql.Config -> Sql.Config) -> Net -> Task Sql.Error Db
connectTo url change net =
    case Sql.config url of
        Ok config ->
            Sql.connect (change config) net

        Err why ->
            Task.fail (Sql.Unreachable why)


{-| The id the server gives the connection a statement runs on.
-}
backend : Db -> Task Sql.Error Int
backend db =
    let
        statement =
            if Database.kind == Postgres then
                "select pg_backend_pid() as id"

            else
                "select connection_id() as id"
    in
    Sql.query statement [] (Sql.field "id" Sql.int) db
        |> Task.map (List.head >> Maybe.withDefault 0)


{-| Ends the server's connection `id` from a connection of its own, and
waits a moment, on the server, for it to go.
-}
kill : Int -> Net -> Task Sql.Error ()
kill id net =
    let
        ( killing, waiting ) =
            if Database.kind == Postgres then
                ( "select pg_terminate_backend(" ++ String.fromInt id ++ ")", "select pg_sleep(0.2)" )

            else
                ( "kill " ++ String.fromInt id, "select sleep(0.2)" )
    in
    connect identity net
        |> Task.andThen (\killer -> Sql.execute killing [] killer |> Task.andThen (\_ -> Sql.execute waiting [] killer))
        |> Task.map (\_ -> ())


count : Db -> Task Sql.Error Int
count =
    Sql.query "select count(*) as n from todos" [] (Sql.field "n" Sql.int)
        >> Task.map (List.head >> Maybe.withDefault -1)


failedKind : Sql.Error -> String
failedKind error =
    case error of
        Sql.Unreachable _ ->
            "unreachable"

        _ ->
            Sql.errorToString error


expectError : Sql.Error -> Task Sql.Error a -> Task x Expect.Expectation
expectError expected task =
    task
        |> Task.map (\_ -> Expect.fail ("succeeded, where it should fail with " ++ Sql.errorToString expected))
        |> Task.onError (\error -> Task.succeed (Expect.equal expected error))


expectUnreachable : Task Sql.Error a -> Task x Expect.Expectation
expectUnreachable task =
    task
        |> Task.map (\_ -> Expect.fail "connected")
        |> Task.onError (\error -> Task.succeed (Expect.equal "unreachable" (failedKind error)))
