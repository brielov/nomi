module LogTest exposing (suite)

import Expect
import Nomi.Fiber as Fiber
import Nomi.Log as Log exposing (Logger)
import Nomi.Test exposing (effect, effectWith)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)


suite : Test
suite =
    describe "Nomi.Log"
        [ effect "a line is JSON, the logger's fields first, then the line's" <|
            \env ->
                Log.info "handled" [ ( "status", Log.int 200 ), ( "ok", Log.bool True ) ] requestLog
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (Expect.equal """{"time":"1970-01-01T00:00:00.000Z","level":"info","message":"handled","request":"r-1","path":"/todos","status":200,"ok":true}
""")
        , effect "a field of a name the logger has replaces its value, where it was" <|
            \env ->
                Log.warn "slow" [ ( "request", Log.string "r-2" ), ( "ms", Log.float 1.5 ) ] requestLog
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (Expect.equal """{"time":"1970-01-01T00:00:00.000Z","level":"warn","message":"slow","request":"r-2","path":"/todos","ms":1.5}
""")
        , effect "a field named as the line's own keys gets a _ before its name" <|
            \env ->
                Log.error "broke" [ ( "message", Log.string "theirs" ), ( "level", Log.int 3 ) ] Log.logger
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (Expect.equal """{"time":"1970-01-01T00:00:00.000Z","level":"error","message":"broke","_message":"theirs","_level":3}
""")
        , effect "a logger writes Info and above, unless it is told otherwise" <|
            \env ->
                Log.debug "hidden" [] Log.logger
                    |> Task.andThen (\_ -> Log.debug "shown" [] (Log.withLevel Log.Debug Log.logger))
                    |> Task.andThen (\_ -> Log.info "hidden too" [] (Log.withLevel Log.Error Log.logger))
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (String.lines >> List.filter (not << String.isEmpty) >> List.length >> Expect.equal 1)
        , effect "a line's time is the program's clock's" <|
            \env ->
                Process.sleep 61001
                    |> Task.andThen (\_ -> Log.info "later" [] Log.logger)
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (String.contains "\"time\":\"1970-01-01T00:01:01.001Z\"" >> Expect.equal True)
        , effect "two loggers, in fibers that take turns, never mix their fields" <|
            \env ->
                [ "a", "b" ]
                    |> List.map (\name -> Fiber.fork (chatter (Log.with [ ( "fiber", Log.string name ) ] Log.logger) name 50))
                    |> Task.sequence
                    |> Task.andThen (List.map Fiber.join >> Task.sequence)
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map
                        (\err ->
                            String.lines err
                                |> List.filter (not << String.isEmpty)
                                |> List.map (\line -> String.contains "\"fiber\":\"a\",\"from\":\"a\"" line || String.contains "\"fiber\":\"b\",\"from\":\"b\"" line)
                                |> (\checks -> ( List.length checks, List.all identity checks ))
                        )
                    |> Task.map (Expect.equal ( 100, True ))
        , effectWith (Nomi.Test.env |> Nomi.Test.withTerminal { columns = 80, rows = 24 }) "on a terminal a line is text, its level coloured" <|
            \env ->
                Log.info "handled" [ ( "status", Log.int 200 ), ( "note", Log.string "two words" ), ( "empty", Log.string "" ) ] requestLog
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (Expect.equal "00:00:00.000 \u{001B}[32mINFO \u{001B}[0m handled request=r-1 path=/todos status=200 note=\"two words\" empty=\"\"\n")
        ]


requestLog : Logger
requestLog =
    Log.with [ ( "request", Log.string "r-1" ), ( "path", Log.string "/todos" ) ] Log.logger


{-| Lines from the fiber, yielding to the other between them.
-}
chatter : Logger -> String -> Int -> Task x ()
chatter log name left =
    if left == 0 then
        Task.succeed ()

    else
        Log.info "tick" [ ( "from", Log.string name ) ] log
            |> Task.andThen (\_ -> Process.sleep 0)
            |> Task.andThen (\_ -> chatter log name (left - 1))
