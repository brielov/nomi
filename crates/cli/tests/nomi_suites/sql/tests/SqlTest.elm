module SqlTest exposing (suite)

import Expect
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs
import Nomi.Sql as Sql exposing (Db, Value(..))
import Nomi.Test exposing (effect)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "Nomi.Sql on SQLite in memory"
        [ statements
        , integers
        , constraints
        , transactions
        , ownership
        ]


{-| A database in memory with a table of todos, the test's to use.
-}
withTodos : (Db -> Task Sql.Error a) -> Task String a
withTodos use =
    Sql.memory
        |> Task.andThen
            (\db ->
                Sql.execute "create table todos (id integer primary key, title text not null unique, done boolean not null default false, note text)" [] db
                    |> Task.andThen (\_ -> use db)
            )
        |> Task.mapError Sql.errorToString


insert : String -> Db -> Task Sql.Error { changes : Int }
insert title =
    Sql.execute "insert into todos (title) values (?)" [ Text title ]


titles : Db -> Task Sql.Error (List String)
titles =
    Sql.query "select title from todos order by id" [] (Sql.field "title" Sql.string)


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
            \_ ->
                withTodos
                    (\db ->
                        Sql.execute "insert into todos (title, done, note) values (?, ?, ?), (?, ?, ?)" [ Text "milk", Bool True, Null, Text "eggs", Bool False, Text "a dozen" ] db
                            |> Task.andThen (\_ -> Sql.query "select * from todos order by id" [] todo db)
                    )
                    |> Task.map (Expect.equal [ Todo 1 "milk" True Nothing, Todo 2 "eggs" False (Just "a dozen") ])
        , effect "every kind of value round-trips" <|
            \_ ->
                let
                    blob =
                        Nomi.Bytes.fromHex "0001feff" |> Maybe.withDefault (Nomi.Bytes.fromString "")
                in
                Sql.memory
                    |> Task.andThen
                        (\db ->
                            Sql.query "select ? as n, ? as f, ? as t, ? as b, ? as z, ? as y" [ Int -42, Float 1.5, Text "é ✓", Blob blob, Null, Bool True ] row db
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal [ ( ( -42, 1.5, "é ✓" ), ( "0001feff", Nothing, True ) ) ])
        , effect "columns gives a row's every column, in the statement's order" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select 2 as b, 'x' as a, null as c" [] Sql.columns)
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal [ [ ( "b", Int 2 ), ( "a", Text "x" ), ( "c", Null ) ] ])
        , effect "execute says how many rows a statement changed" <|
            \_ ->
                withTodos
                    (\db ->
                        insert "milk" db
                            |> Task.andThen (\_ -> insert "eggs" db)
                            |> Task.andThen (\_ -> Sql.execute "update todos set done = ?" [ Bool True ] db)
                    )
                    |> Task.map (Expect.equal { changes = 2 })
        , effect "a script without parameters runs every statement" <|
            \_ ->
                Sql.memory
                    |> Task.andThen
                        (\db ->
                            Sql.execute "create table a (x); create table b (y); insert into b values (1)" [] db
                                |> Task.andThen (\_ -> Sql.query "select y from b" [] (Sql.field "y" Sql.int) db)
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal [ 1 ])
        , effect "a row the decoder can't read fails with which, and why" <|
            \_ ->
                withTodos (\db -> insert "milk" db |> Task.andThen (\_ -> Sql.query "select * from todos" [] (Sql.field "note" Sql.string) db))
                    |> expectError "row 1: column \"note\": expected text, got NULL"
        , effect "a missing column fails by name" <|
            \_ ->
                withTodos (\db -> insert "milk" db |> Task.andThen (\_ -> Sql.query "select id from todos" [] (Sql.field "title" Sql.string) db))
                    |> expectError "row 1: no column \"title\""
        , effect "a statement the database refuses fails with its code" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select * from nowhere" [] Sql.value)
                    |> Task.map (\_ -> Expect.fail "succeeded")
                    |> Task.onError (\error -> Task.succeed (Expect.equal (Sql.Failed { code = "SQLITE_ERROR", message = "no such table: nowhere" }) error))
        , effect "a NaN is refused, as SQLite would store NULL" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select ? as f" [ Float (0 / 0) ] Sql.value)
                    |> expectSqlError (Sql.BadParameter 1 "NaN, which SQLite stores as NULL")
        ]


row : Sql.Decoder ( ( Int, Float, String ), ( String, Maybe Int, Bool ) )
row =
    Sql.map2 Tuple.pair
        (Sql.map3 (\a b c -> ( a, b, c )) (Sql.field "n" Sql.int) (Sql.field "f" Sql.float) (Sql.field "t" Sql.string))
        (Sql.map3 (\a b c -> ( a, b, c )) (Sql.field "b" (Sql.map Nomi.Bytes.toHex Sql.bytes)) (Sql.field "z" (Sql.nullable Sql.int)) (Sql.field "y" Sql.bool))


integers : Test
integers =
    describe "64-bit integers"
        [ effect "an integer past 2^53 round-trips exactly through Int64 and int64" <|
            \_ ->
                Sql.memory
                    |> Task.andThen
                        (\db ->
                            Sql.execute "create table big (n integer)" [] db
                                |> Task.andThen (\_ -> Sql.execute "insert into big values (?), (?), (?)" [ Int64 "9007199254740993", Int64 "9223372036854775807", Int64 "-9223372036854775808" ] db)
                                |> Task.andThen (\_ -> Sql.query "select n from big order by rowid" [] (Sql.field "n" Sql.int64) db)
                        )
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal [ "9007199254740993", "9223372036854775807", "-9223372036854775808" ])
        , effect "one past 2^53 fails int rather than round" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select ? as n" [ Int64 "9007199254740993" ] (Sql.field "n" Sql.int))
                    |> expectSqlError (Sql.BadRow "row 1: column \"n\": 9007199254740993 is past ±(2^53 − 1), the integers an Int holds exactly: read it with int64")
        , effect "the largest exact Int reads as an Int" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select ? as n, ? as m" [ Int 9007199254740991, Int -9007199254740991 ] (Sql.map2 Tuple.pair (Sql.field "n" Sql.int) (Sql.field "m" Sql.int)))
                    |> Task.mapError Sql.errorToString
                    |> Task.map (Expect.equal [ ( 9007199254740991, -9007199254740991 ) ])
        , effect "an Int past 2^53, already rounded, is refused as a parameter" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select ? as n" [ Int 9007199254740993 ] Sql.value)
                    |> expectSqlError (Sql.BadParameter 1 "9007199254740992 is past ±(2^53 − 1), where an Int is rounded: pass it with Int64")
        , effect "Int64 takes only a 64-bit integer in decimal" <|
            \_ ->
                [ "9223372036854775808", "1.5", "", "0x10" ]
                    |> List.map
                        (\text ->
                            Sql.memory
                                |> Task.andThen (Sql.query "select ? as n" [ Text "ok", Int64 text ] Sql.value)
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
            \_ ->
                Sql.memory
                    |> Task.andThen
                        (\db ->
                            Sql.execute "create table lists (id integer primary key); create table items (id integer primary key, list integer references lists (id), name text not null unique, qty integer check (qty > 0))" [] db
                                |> Task.andThen (\_ -> Sql.execute "insert into lists values (1); insert into items values (1, 1, 'a', 1)" [] db)
                                |> Task.andThen
                                    (\_ ->
                                        [ "insert into items values (2, 1, 'a', 1)"
                                        , "insert into items values (1, 1, 'b', 1)"
                                        , "insert into items values (2, 9, 'b', 1)"
                                        , "insert into items values (2, 1, null, 1)"
                                        , "insert into items values (2, 1, 'b', 0)"
                                        ]
                                            |> List.map (\statement -> Sql.execute statement [] db |> Task.map (\_ -> Nothing) |> Task.onError (violation >> Task.succeed))
                                            |> Task.sequence
                                    )
                        )
                    |> Task.mapError Sql.errorToString
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
    = Database Sql.Error
    | Mine String


transactions : Test
transactions =
    describe "transactions"
        [ effect "a transaction that succeeds commits" <|
            \_ ->
                withTodos
                    (\db ->
                        Sql.transaction Database (\tx -> insert "milk" tx |> Task.andThen (\_ -> insert "eggs" tx) |> Task.mapError Database) db
                            |> Task.mapError (\_ -> Sql.Closed)
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "milk", "eggs" ])
        , effect "a database error rolls it back" <|
            \_ ->
                rolledBack (\tx -> insert "milk" tx |> Task.andThen (\_ -> insert "milk" tx) |> Task.mapError Database)
                    |> Task.map (Expect.equal ( [ "kept" ], "database" ))
        , effect "the program's own error rolls it back" <|
            \_ ->
                rolledBack (\tx -> insert "milk" tx |> Task.mapError Database |> Task.andThen (\_ -> Task.fail (Mine "no")))
                    |> Task.map (Expect.equal ( [ "kept" ], "mine" ))
        , effect "an interruption rolls it back" <|
            \_ ->
                withTodos
                    (\db ->
                        insert "kept" db
                            |> Task.andThen
                                (\_ ->
                                    Fiber.fork (Sql.transaction Database (\tx -> insert "milk" tx |> Task.mapError Database |> Task.andThen (\_ -> Process.sleep 1000)) db)
                                        |> Task.andThen (\fiber -> Process.sleep 500 |> Task.andThen (\_ -> Fiber.interrupt fiber))
                                )
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "kept" ])
        , effect "while a transaction runs, other fibers' statements wait for it" <|
            \_ ->
                withTodos
                    (\db ->
                        Fiber.fork (Sql.transaction Database (\tx -> insert "milk" tx |> Task.mapError Database |> Task.andThen (\_ -> Process.sleep 1000) |> Task.andThen (\_ -> insert "eggs" tx |> Task.mapError Database)) db)
                            |> Task.andThen (\fiber -> Process.sleep 10 |> Task.andThen (\_ -> titles db) |> Task.map (Tuple.pair fiber))
                            |> Task.andThen (\( fiber, seen ) -> Fiber.join fiber |> Task.mapError (\_ -> Sql.Closed) |> Task.map (\_ -> seen))
                    )
                    |> Task.map (Expect.equal [ "milk", "eggs" ])
        , effect "a transaction within one is a savepoint, which rolls back alone" <|
            \_ ->
                withTodos
                    (\db ->
                        Sql.transaction Database
                            (\tx ->
                                insert "milk" tx
                                    |> Task.mapError Database
                                    |> Task.andThen (\_ -> Sql.transaction Database (\inner -> insert "eggs" inner |> Task.mapError Database |> Task.andThen (\_ -> Task.fail (Mine "no"))) tx |> Task.onError (\_ -> Task.succeed ()))
                                    |> Task.andThen (\_ -> insert "bread" tx |> Task.mapError Database)
                            )
                            db
                            |> Task.mapError (\_ -> Sql.Closed)
                            |> Task.andThen (\_ -> titles db)
                    )
                    |> Task.map (Expect.equal [ "milk", "bread" ])
        , effect "a transaction's Db is closed once it ends" <|
            \_ ->
                withTodos (\db -> Sql.transaction Database Task.succeed db |> Task.mapError (\_ -> Sql.Closed) |> Task.andThen (insert "late"))
                    |> expectError "the database is closed"
        ]


{-| The titles after "kept" was inserted, then the transaction ran and
failed, and which error it failed with.
-}
rolledBack : (Db -> Task Failure a) -> Task String ( List String, String )
rolledBack run =
    withTodos
        (\db ->
            insert "kept" db
                |> Task.andThen
                    (\_ ->
                        Sql.transaction Database run db
                            |> Task.map (\_ -> "committed")
                            |> Task.onError
                                (\failure ->
                                    Task.succeed
                                        (case failure of
                                            Database _ ->
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
            \_ ->
                Fiber.fork Sql.memory
                    |> Task.andThen Fiber.join
                    |> Task.andThen (Sql.query "select 1 as one" [] Sql.value)
                    |> expectSqlError Sql.Closed
        , effect "closing it wakes the statements waiting for a transaction, which find it closed" <|
            \_ ->
                withTodos
                    (\db ->
                        Fiber.fork (Sql.transaction Database (\_ -> Process.sleep 1000) db)
                            |> Task.andThen (\_ -> Fiber.fork (Process.sleep 10 |> Task.andThen (\_ -> titles db) |> Task.map (\_ -> "read") |> Task.onError (\error -> Time.now |> Task.map (\time -> Sql.errorToString error ++ " at " ++ String.fromInt (Time.posixToMillis time)))))
                            |> Task.andThen (\waiting -> Process.sleep 20 |> Task.andThen (\_ -> Sql.close db) |> Task.andThen (\_ -> Fiber.join waiting))
                    )
                    |> Task.map (Expect.equal "the database is closed at 20")
        , effect "close closes it" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (\db -> Sql.close db |> Task.andThen (\_ -> Sql.query "select 1 as one" [] Sql.value db))
                    |> expectSqlError Sql.Closed
        , effect "a file system with no disk holds no database" <|
            \env ->
                Sql.open "todos.db" env.fs
                    |> expectSqlError (Sql.NotOnDisk "todos.db")
        , effect "a path outside the capability's root fails as Fs does" <|
            \env ->
                Sql.open "../todos.db" env.fs
                    |> expectSqlError (Sql.FileError (Fs.OutsideRoot "../todos.db"))
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
