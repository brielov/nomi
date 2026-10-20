module Database exposing (Kind(..), kind, fresh, sql, autoId, blobType)

{-| The database the shared tests run on: SQLite in memory here, and a
server's in what `cargo xtask services` writes in this module's place.
-}

import Nomi.Net exposing (Net)
import Nomi.Sql as Sql exposing (Db)
import Task exposing (Task)


type Kind
    = Sqlite
    | Postgres
    | Mysql


kind : Kind
kind =
    Sqlite


{-| A database holding none of the tests' tables.
-}
fresh : Net -> Task Sql.Error Db
fresh _ =
    Sql.memory


{-| A statement with its `?`s as this database's placeholders.
-}
sql : String -> String
sql =
    identity


{-| An integer key's column, which the database numbers itself.
-}
autoId : String
autoId =
    "integer primary key"


blobType : String
blobType =
    "blob"
