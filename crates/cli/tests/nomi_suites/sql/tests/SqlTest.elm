module SqlTest exposing (suite)

{-| What `Nomi.Sql` does on every database: on SQLite in memory here, and
on Postgres and MySQL under `cargo xtask services`, which gives `Database`
their servers.
-}

import Database
import Expect
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Net exposing (Net)
import Nomi.Sql as Sql exposing (Db, Value(..))
import Nomi.Test exposing (effect)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Sql"
        [ statements
        , integers
        , constraints
        , transactions
        , ownership
        ]


query : String -> List Value -> Sql.Decoder a -> Db -> Task Sql.Error (List a)
query statement =
    Sql.query (Database.sql statement)


execute : String -> List Value -> Db -> Task Sql.Error { changes : Int }
execute statement =
    Sql.execute (Database.sql statement)


{-| A fresh database with a table of todos, the test's to use.
-}
withTodos : Net -> (Db -> Task Sql.Error a) -> Task String a
withTodos net use =
    Database.fresh net
        |> Task.andThen
            (\db ->
                execute ("create table todos (id " ++ Database.autoId ++ ", title varchar(100) not null unique, done boolean not null default false, note varchar(100))") [] db
                    |> Task.andThen (\_ -> use db)
            )
        |> Task.mapError Sql.errorToString


{-| A fresh database, the test's to use.
-}
with : Net -> (Db -> Task Sql.Error a) -> Task String a
with net use =
    Database.fresh net
        |> Task.andThen use
        |> Task.mapError Sql.errorToString


insert : String -> Db -> Task Sql.Error { changes : Int }
insert title =
    execute "insert into todos (title) values (?)" [ Text title ]


titles : Db -> Task Sql.Error (List String)
titles =
    query "select title from todos order by id" [] (Sql.field "title" Sql.string)


type alias Todo =
    { id : Int, title : String, done : Bool, note : Maybe String }


todo : Sql.Decoder Todo
todo =
    Sql.map4 Todo
        (Sql.field "id" Sql.int)
        (Sql.field "title" Sql.string)
        (Sql.field "done" Sql.bool)
        (Sql.field "note" (Sql.nullable Sql.string))


statements : Test
statements =
    describe "statements"
        [ effect "rows are read by column name" <|
            \env ->
                withTodos env.net
                    (\db ->
                        execute "insert into todos (title, done, note) values (?, ?, ?), (?, ?, ?)" [ Text "milk", Bool True, Null, Text "eggs", Bool False, Text "a dozen" ] db
                            |> Task.andThen (\_ -> query "select * from todos order by id" [] todo db)
                    )
                    |> Task.map (Expect.equal [ Todo 1 "milk" True Nothing, Todo 2 "eggs" False (Just "a dozen") ])
        , effect "every kind of value round-trips" <|
            \env ->
                let
                    blob =
                        Nomi.Bytes.fromHex "0001feff" |> Maybe.withDefault (Nomi.Bytes.fromString "")
                in
                with env.net
                    (\db ->
                        execute ("create table kinds (n bigint, f double precision, t varchar(100), b " ++ Database.blobType ++ ", z integer, y boolean)") [] db
                            |> Task.andThen (\_ -> execute "insert into kinds values (?, ?, ?, ?, ?, ?)" [ Int -42, Float 1.5, Text "é ✓", Blob blob, Null, Bool True ] db)
                            |> Task.andThen (\_ -> query "select * from kinds" [] row db)
                    )
                    |> Task.map (Expect.equal [ ( ( -42, 1.5, "é ✓" ), ( "0001feff", Nothing, True ) ) ])
        , effect "columns gives a row's every column, in the statement's order" <|
            \env ->
                with env.net (query "select 2 as b, 'x' as a, null as c" [] Sql.columns)
                    |> Task.map (Expect.equal [ [ ( "b", Int 2 ), ( "a", Text "x" ), ( "c", Null ) ] ])
        , effect "execute says how many rows a statement changed" <|
            \env ->
                withTodos env.net
                    (\db ->
                        insert "milk" db
                            |> Task.andThen (\_ -> insert "eggs" db)
                            |> Task.andThen (\_ -> execute "update todos set done = ?" [ Bool True ] db)
                    )
                    |> Task.map (Expect.equal { changes = 2 })
        , effect "a script without parameters runs every statement" <|
            \env ->
                with env.net
                    (\db ->
                        execute "create table a (x integer); create table b (y integer); insert into b values (1)" [] db
                            |> Task.andThen (\_ -> query "select y from b" [] (Sql.field "y" Sql.int) db)
                    )
                    |> Task.map (Expect.equal [ 1 ])
        , effect "a row the decoder can't read fails with which, and why" <|
            \env ->
                withTodos env.net (\db -> insert "milk" db |> Task.andThen (\_ -> query "select * from todos" [] (Sql.field "note" Sql.string) db))
                    |> expectError "row 1: column \"note\": expected text, got NULL"
        , effect "a missing column fails by name" <|
            \env ->
                withTodos env.net (\db -> insert "milk" db |> Task.andThen (\_ -> query "select id from todos" [] (Sql.field "title" Sql.string) db))
                    |> expectError "row 1: no column \"title\""
        , effect "a statement the database refuses fails with its code and message" <|
            \env ->
                Database.fresh env.net
                    |> Task.andThen (query "select * from nowhere" [] Sql.value)
                    |> Task.map (\_ -> Expect.fail "succeeded")
                    |> Task.onError
                        (\error ->
                            Task.succeed
                                (case error of
                                    Sql.Failed { code, message } ->
                                        Expect.equal ( True, True ) ( code /= "", String.contains "nowhere" message )

                                    _ ->
                                        Expect.fail (Sql.errorToString error)
                                )
                        )
        ]


row : Sql.Decoder ( ( Int, Float, String ), ( String, Maybe Int, Bool ) )
row =
    Sql.map2 Tuple.pair
        (Sql.map3 (\a b c -> ( a, b, c )) (Sql.field "n" Sql.int) (Sql.field "f" Sql.float) (Sql.field "t" Sql.string))
        (Sql.map3 (\a b c -> ( a, b, c )) (Sql.field "b" (Sql.map Nomi.Bytes.toHex Sql.bytes)) (Sql.field "z" (Sql.nullable Sql.int)) (Sql.field "y" Sql.bool))


{-| A fresh database with a table of 64-bit integers holding these.
-}
withBig : Net -> List Value -> (Db -> Task Sql.Error a) -> Task String a
withBig net values use =
    with net
        (\db ->
            execute ("create table big (id " ++ Database.autoId ++ ", n bigint)") [] db
                |> Task.andThen (\_ -> Task.sequence (List.map (\n -> execute "insert into big (n) values (?)" [ n ] db) values))
                |> Task.andThen (\_ -> use db)
        )


integers : Test
integers =
    describe "64-bit integers"
        [ effect "an integer past 2^53 round-trips exactly through Int64 and int64" <|
            \env ->
                withBig env.net
                    [ Int64 "9007199254740993", Int64 "9223372036854775807", Int64 "-9223372036854775808" ]
                    (query "select n from big order by id" [] (Sql.field "n" Sql.int64))
                    |> Task.map (Expect.equal [ "9007199254740993", "9223372036854775807", "-9223372036854775808" ])
        , effect "one past 2^53 fails int rather than round" <|
            \env ->
                withBig env.net [ Int64 "9007199254740993" ] (query "select n from big" [] (Sql.field "n" Sql.int))
                    |> expectError "row 1: column \"n\": 9007199254740993 is past ±(2^53 − 1), the integers an Int holds exactly: read it with int64"
        , effect "the largest exact Int reads as an Int" <|
            \env ->
                withBig env.net [ Int 9007199254740991, Int -9007199254740991 ] (query "select n from big order by id" [] (Sql.field "n" Sql.int))
                    |> Task.map (Expect.equal [ 9007199254740991, -9007199254740991 ])
        , effect "an Int past 2^53, already rounded, is refused as a parameter" <|
            \env ->
                Database.fresh env.net
                    |> Task.andThen (query "select ? as n" [ Int 9007199254740993 ] Sql.value)
                    |> expectSqlError (Sql.BadParameter 1 "9007199254740992 is past ±(2^53 − 1), where an Int is rounded: pass it with Int64")
        , effect "Int64 takes only a 64-bit integer in decimal" <|
            \env ->
                [ "9223372036854775808", "1.5", "", "0x10" ]
                    |> List.map
                        (\text ->
                            Database.fresh env.net
                                |> Task.andThen (query "select ? as n, ? as m" [ Text "ok", Int64 text ] Sql.value)
                                |> Task.map (\_ -> "accepted " ++ text)
                                |> Task.onError (\error -> Task.succeed (Sql.errorToString error))
                        )
                    |> Task.sequence
                    |> Task.map
                        (Expect.equal
                            [ "parameter 2: \"9223372036854775808\" is not a 64-bit integer in decimal"
                            , "parameter 2: \"1.5\" is not a 64-bit integer in decimal"
                            , "parameter 2: \"\" is not a 64-bit integer in decimal"
                            , "parameter 2: \"0x10\" is not a 64-bit integer in decimal"
                            ]
                        )
        ]


constraints : Test
constraints =
    describe "constraints"
        [ effect "each kind of constraint a change breaks is told apart" <|
            \env ->
                with env.net
                    (\db ->
                        execute "create table lists (id integer primary key)" [] db
                            |> Task.andThen (\_ -> execute "create table items (id integer primary key, list integer, name varchar(20) not null unique, qty integer check (qty > 0), foreign key (list) references lists (id))" [] db)
                            |> Task.andThen (\_ -> execute "insert into lists values (1)" [] db)
                            |> Task.andThen (\_ -> execute "insert into items values (1, 1, 'a', 1)" [] db)
                            |> Task.andThen
                                (\_ ->
                                    [ "insert into items values (2, 1, 'a', 1)"
                                    , "insert into items values (1, 1, 'b', 1)"
                                    , "insert into items values (2, 9, 'b', 1)"
                                    , "insert into items values (2, 1, null, 1)"
                                    , "insert into items values (2, 1, 'b', 0)"
                                    ]
                                        |> List.map (\statement -> execute statement [] db |> Task.map (\_ -> Nothing) |> Task.onError (violation >> Task.succeed))
                                        |> Task.sequence
                                )
                    )
                    |> Task.map (Expect.equal [ Just Sql.Unique, Just Sql.Unique, Just Sql.ForeignKey, Just Sql.NotNull, Just Sql.Check ])
        ]


violation : Sql.Error -> Maybe Sql.Constraint
violation error =
    case error of
        Sql.Violated constraint _ ->
            Just constraint

        _ ->
            Nothing


type Failure
    = FromDatabase Sql.Error
    | Mine String


transactions : Test
transactions =
    describe "transactions"
        [ effect "a transaction that succeeds commits" <|
            \env ->
                withTodos env.net
                    (\db ->
                        Sql.transaction FromDatabase (\tx -> insert "milk" tx |> Task.andThen (\_ -> insert "eggs" tx) |> Task.mapError FromDatabase) db
                            |> Task.mapError (\_ -> Sql.Closed)
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "milk", "eggs" ])
        , effect "a database error rolls it back" <|
            \env ->
                rolledBack env.net (\tx -> insert "milk" tx |> Task.andThen (\_ -> insert "milk" tx) |> Task.mapError FromDatabase)
                    |> Task.map (Expect.equal ( [ "kept" ], "database" ))
        , effect "the program's own error rolls it back" <|
            \env ->
                rolledBack env.net (\tx -> insert "milk" tx |> Task.mapError FromDatabase |> Task.andThen (\_ -> Task.fail (Mine "no")))
                    |> Task.map (Expect.equal ( [ "kept" ], "mine" ))
        , effect "an interruption rolls it back" <|
            \env ->
                withTodos env.net
                    (\db ->
                        insert "kept" db
                            |> Task.andThen
                                (\_ ->
                                    Fiber.fork (Sql.transaction FromDatabase (\tx -> insert "milk" tx |> Task.mapError FromDatabase |> Task.andThen (\_ -> Process.sleep 1000)) db)
                                        |> Task.andThen (\fiber -> Process.sleep 500 |> Task.andThen (\_ -> Fiber.interrupt fiber))
                                )
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "kept" ])
        , effect "a transaction within one is a savepoint, which rolls back alone" <|
            \env ->
                withTodos env.net
                    (\db ->
                        Sql.transaction FromDatabase
                            (\tx ->
                                insert "milk" tx
                                    |> Task.mapError FromDatabase
                                    |> Task.andThen (\_ -> Sql.transaction FromDatabase (\inner -> insert "eggs" inner |> Task.mapError FromDatabase |> Task.andThen (\_ -> Task.fail (Mine "no"))) tx |> Task.onError (\_ -> Task.succeed ()))
                                    |> Task.andThen (\_ -> insert "bread" tx |> Task.mapError FromDatabase)
                            )
                            db
                            |> Task.mapError (\_ -> Sql.Closed)
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "milk", "bread" ])
        , effect "a transaction's Db is closed once it ends" <|
            \env ->
                withTodos env.net (\db -> Sql.transaction FromDatabase Task.succeed db |> Task.mapError (\_ -> Sql.Closed) |> Task.andThen (insert "late"))
                    |> expectError "the database is closed"
        ]


{-| The titles after "kept" was inserted, then the transaction ran and
failed, and which error it failed with.
-}
rolledBack : Net -> (Db -> Task Failure a) -> Task String ( List String, String )
rolledBack net run =
    withTodos net
        (\db ->
            insert "kept" db
                |> Task.andThen
                    (\_ ->
                        Sql.transaction FromDatabase run db
                            |> Task.map (\_ -> "committed")
                            |> Task.onError
                                (\failure ->
                                    Task.succeed
                                        (case failure of
                                            FromDatabase _ ->
                                                "database"

                                            Mine _ ->
                                                "mine"
                                        )
                                )
                    )
                |> Task.andThen (\how -> titles db |> Task.map (\all -> ( all, how )))
        )


ownership : Test
ownership =
    describe "ownership"
        [ effect "a database closes once the fiber that opened it ends" <|
            \env ->
                Fiber.fork (Database.fresh env.net)
                    |> Task.andThen Fiber.join
                    |> Task.andThen (query "select 1 as one" [] Sql.value)
                    |> expectSqlError Sql.Closed
        , effect "close closes it" <|
            \env ->
                Database.fresh env.net
                    |> Task.andThen (\db -> Sql.close db |> Task.andThen (\_ -> query "select 1 as one" [] Sql.value db))
                    |> expectSqlError Sql.Closed
        ]


expectError : String -> Task String a -> Task String Expect.Expectation
expectError expected task =
    task
        |> Task.map (\_ -> Expect.fail ("succeeded, where it should fail with " ++ expected))
        |> Task.onError (\error -> Task.succeed (Expect.equal expected error))


expectSqlError : Sql.Error -> Task Sql.Error a -> Task String Expect.Expectation
expectSqlError expected task =
    task
        |> Task.map (\_ -> Expect.fail ("succeeded, where it should fail with " ++ Sql.errorToString expected))
        |> Task.onError (\error -> Task.succeed (Expect.equal expected error))
