module Nomi.Log exposing
    ( Logger, logger, with, withLevel, Level(..)
    , debug, info, warn, error
    , Value, string, int, float, bool
    )

{-| Logging, with a logger a program passes to what logs, as it passes any
dependency: fields it carries, as a request's id, are on every line it
writes, and two loggers never share them.

    handle : Logger -> Request -> Task Never Response
    handle log req =
        let
            requestLog =
                Nomi.Log.with [ ( "path", Nomi.Log.string (String.join "/" (Server.path req)) ) ] log
        in
        Nomi.Log.info "handled" [ ( "status", Nomi.Log.int 200 ) ] requestLog
            |> Task.map (\_ -> Server.text 200 "ok")

A line goes to the standard error of whatever runs the task: the
process's, or under `nomi test` the test's, which `Nomi.Test.stderr`
reads. On a terminal it is text for a person; anywhere else it is a line
of JSON, with the line's `time`, `level` and `message` and then each
field, for whatever reads the logs. Its time is the program's clock's.

@docs Logger, logger, with, withLevel, Level


# Writing

@docs debug, info, warn, error


# Values

@docs Value, string, int, float, bool

-}

import Elm.Kernel.NomiLog
import Task exposing (Task)


{-| What writes log lines: the least level it writes, and the fields on
each line.
-}
type Logger
    = Logger
        { level : Level
        , fields : List ( String, Value )
        }


{-| A line's level: detail for finding a fault, what happened, what may be
wrong, and what went wrong.
-}
type Level
    = Debug
    | Info
    | Warn
    | Error


{-| A field's value.
-}
type Value
    = Value Primitive


type Primitive
    = Text String
    | Number Float
    | Whole Int
    | Truth Bool


{-| A logger writing lines of `Info` and above, with no fields.
-}
logger : Logger
logger =
    Logger { level = Info, fields = [] }


{-| The logger, with these fields on each line besides its own. A field of
a name it has already takes the new value, where the old one was.
-}
with : List ( String, Value ) -> Logger -> Logger
with fields (Logger l) =
    Logger { l | fields = merge l.fields fields }


{-| The logger, writing lines of the level and above: `Debug` writes them
all.
-}
withLevel : Level -> Logger -> Logger
withLevel level (Logger l) =
    Logger { l | level = level }


merge : List ( String, Value ) -> List ( String, Value ) -> List ( String, Value )
merge base added =
    List.foldl
        (\( name, value ) fields ->
            if List.any (\( key, _ ) -> key == name) fields then
                List.map
                    (\( key, old ) ->
                        if key == name then
                            ( key, value )

                        else
                            ( key, old )
                    )
                    fields

            else
                fields ++ [ ( name, value ) ]
        )
        base
        added



-- WRITING


{-| Writes a line of detail, for finding a fault.
-}
debug : String -> List ( String, Value ) -> Logger -> Task x ()
debug =
    write Debug


{-| Writes a line of what happened.
-}
info : String -> List ( String, Value ) -> Logger -> Task x ()
info =
    write Info


{-| Writes a line of what may be wrong.
-}
warn : String -> List ( String, Value ) -> Logger -> Task x ()
warn =
    write Warn


{-| Writes a line of what went wrong.
-}
error : String -> List ( String, Value ) -> Logger -> Task x ()
error =
    write Error


write : Level -> String -> List ( String, Value ) -> Logger -> Task x ()
write level message fields (Logger l) =
    if rank level < rank l.level then
        Task.succeed ()

    else
        Elm.Kernel.NomiLog.write (levelName level) message (List.map (\( name, Value value ) -> ( name, value )) (merge l.fields fields))


rank : Level -> Int
rank level =
    case level of
        Debug ->
            0

        Info ->
            1

        Warn ->
            2

        Error ->
            3


levelName : Level -> String
levelName level =
    case level of
        Debug ->
            "debug"

        Info ->
            "info"

        Warn ->
            "warn"

        Error ->
            "error"



-- VALUES


{-| Text.
-}
string : String -> Value
string =
    Value << Text


{-| A whole number.
-}
int : Int -> Value
int =
    Value << Whole


{-| A number.
-}
float : Float -> Value
float =
    Value << Number


{-| True or false.
-}
bool : Bool -> Value
bool =
    Value << Truth
