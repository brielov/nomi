module SqliteTest exposing (suite)

{-| What `Nomi.Sql` does on SQLite alone: its one connection, which a
transaction holds, its codes, its NULL for NaN, and its files.
-}

import Expect
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
    describe "Nomi.Sql on SQLite"
        [ effect "a statement SQLite refuses fails with its code" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select * from nowhere" [] Sql.value)
                    |> expectError (Sql.Failed { code = "SQLITE_ERROR", message = "no such table: nowhere" })
        , effect "a NaN is refused, as SQLite would store NULL" <|
            \_ ->
                Sql.memory
                    |> Task.andThen (Sql.query "select ? as f" [ Float (0 / 0) ] Sql.value)
                    |> expectError (Sql.BadParameter 1 "NaN, which SQLite stores as NULL")
        , effect "while a transaction runs, other fibers' statements wait for it" <|
            \_ ->
                withTodos
                    (\db ->
                        Fiber.fork (Sql.transaction identity (\tx -> insert "milk" tx |> Task.andThen (\_ -> Process.sleep 1000) |> Task.andThen (\_ -> insert "eggs" tx)) db)
                            |> Task.andThen (\fiber -> Process.sleep 10 |> Task.andThen (\_ -> titles db) |> Task.map (Tuple.pair fiber))
                            |> Task.andThen (\( fiber, seen ) -> Fiber.join fiber |> Task.map (\_ -> seen))
                    )
                    |> Task.map (Expect.equal (Ok [ "milk", "eggs" ]))
        , effect "closing it wakes the statements waiting for a transaction, which find it closed" <|
            \_ ->
                withTodos
                    (\db ->
                        Fiber.fork (Sql.transaction identity (\_ -> Process.sleep 1000) db)
                            |> Task.andThen (\_ -> Fiber.fork (Process.sleep 10 |> Task.andThen (\_ -> titles db) |> Task.map (\_ -> "read") |> Task.onError (\error -> Time.now |> Task.map (\time -> Sql.errorToString error ++ " at " ++ String.fromInt (Time.posixToMillis time)))))
                            |> Task.andThen (\waiting -> Process.sleep 20 |> Task.andThen (\_ -> Sql.close db) |> Task.andThen (\_ -> Fiber.join waiting))
                    )
                    |> Task.map (Expect.equal (Ok "the database is closed at 20"))
        , effect "a file system with no disk holds no database" <|
            \env ->
                Sql.open "todos.db" env.fs
                    |> expectError (Sql.NotOnDisk "todos.db")
        , effect "a path outside the capability's root fails as Fs does" <|
            \env ->
                Sql.open "../todos.db" env.fs
                    |> expectError (Sql.FileError (Fs.OutsideRoot "../todos.db"))
        ]


{-| A database in memory with a table of todos, the test's to use, and
whether the task failed.
-}
withTodos : (Db -> Task Sql.Error a) -> Task x (Result Sql.Error a)
withTodos use =
    Sql.memory
        |> Task.andThen
            (\db ->
                Sql.execute "create table todos (id integer primary key, title text not null)" [] db
                    |> Task.andThen (\_ -> use db)
            )
        |> Task.map Ok
        |> Task.onError (Err >> Task.succeed)


insert : String -> Db -> Task Sql.Error { changes : Int }
insert title =
    Sql.execute "insert into todos (title) values (?)" [ Text title ]


titles : Db -> Task Sql.Error (List String)
titles =
    Sql.query "select title from todos order by id" [] (Sql.field "title" Sql.string)


expectError : Sql.Error -> Task Sql.Error a -> Task x Expect.Expectation
expectError expected task =
    task
        |> Task.map (\_ -> Expect.fail ("succeeded, where it should fail with " ++ Sql.errorToString expected))
        |> Task.onError (\error -> Task.succeed (Expect.equal expected error))
