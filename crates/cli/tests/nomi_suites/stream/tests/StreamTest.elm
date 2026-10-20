module StreamTest exposing (suite)

import Bytes exposing (Bytes)
import Bytes.Encode as Encode
import Expect
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Ref as Ref
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream exposing (Reader)
import Nomi.Task
import Nomi.Test exposing (effect, effectWith)
import Process
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "Nomi.Stream"
        [ reading
        , lines
        , writing
        , stdio
        ]



-- READING


reading : Test
reading =
    describe "reading"
        [ effect "a slow writer holds back its producer" <|
            \_ ->
                Ref.make []
                    |> Task.andThen
                        (\log ->
                            let
                                note line =
                                    Ref.update ((::) line) log
                            in
                            Ref.make 0
                                |> Task.andThen
                                    (\count ->
                                        Stream.reader
                                            (Ref.modify (\n -> ( n, n + 1 )) count
                                                |> Task.andThen
                                                    (\n ->
                                                        if n < 3 then
                                                            note ("gave " ++ String.fromInt n) |> Task.map (\_ -> Just (Nomi.Bytes.fromString (String.fromInt n)))

                                                        else
                                                            Task.succeed Nothing
                                                    )
                                            )
                                    )
                                |> Task.andThen
                                    (\from ->
                                        Stream.writer
                                            { write = \bytes -> Process.sleep 1000 |> Task.andThen (\_ -> note ("wrote " ++ text bytes))
                                            , close = Task.succeed ()
                                            }
                                            |> Task.andThen (Stream.pipe from)
                                    )
                                |> Task.andThen (\_ -> Task.map2 Tuple.pair (Ref.get log) Time.now)
                        )
                    |> Task.mapError Stream.errorToString
                    |> Task.map
                        (Expect.equal
                            ( [ "wrote 2", "gave 2", "wrote 1", "gave 1", "wrote 0", "gave 0" ]
                            , Time.millisToPosix 3000
                            )
                        )
        , effect "an interrupted read loses nothing" <|
            \_ ->
                slowChunks 1000 [ "a", "b" ]
                    |> Task.andThen
                        (\from ->
                            Nomi.Task.timeout 500 (Stream.read from)
                                |> Task.andThen
                                    (\first ->
                                        Process.sleep 1000
                                            |> Task.andThen (\_ -> Task.map2 Tuple.pair (readText from) (readText from))
                                            |> Task.map (Tuple.pair (Maybe.map (Maybe.map text) first))
                                    )
                        )
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal ( Nothing, ( Just "a", Just "b" ) ))
        , effect "concurrent reads of a task's reader are answered in order" <|
            \_ ->
                slowChunks 100 [ "a", "b", "c" ]
                    |> Task.andThen (\from -> concurrently (List.repeat 4 (readText from)))
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ Just "a", Just "b", Just "c", Nothing ])
        , effect "readAll and limit fail past their limit" <|
            \_ ->
                Task.sequence
                    [ chunks [ "ab", "cd" ] |> Task.andThen (Stream.readAll { limit = 4 }) |> Task.map text |> result
                    , chunks [ "ab", "cd" ] |> Task.andThen (Stream.readAll { limit = 3 }) |> Task.map text |> result
                    , chunks [ "ab", "cd" ]
                        |> Task.andThen (Stream.limit 3)
                        |> Task.andThen (\from -> readText from |> Task.andThen (\first -> readText from |> Task.map (\second -> String.join "," (List.filterMap identity [ first, second ]))))
                        |> result
                    ]
                    |> Task.map (Expect.equal [ Ok "abcd", Err Stream.TooLarge, Err Stream.TooLarge ])
        , effect "a source that fails fails every read after, running once" <|
            \_ ->
                Ref.make 0
                    |> Task.andThen
                        (\runs ->
                            Stream.reader (Ref.update ((+) 1) runs |> Task.andThen (\_ -> Task.fail Stream.TimedOut))
                                |> Task.andThen (\from -> Task.sequence [ result (Stream.read from), result (Stream.read from) ])
                                |> Task.andThen (\reads -> Ref.get runs |> Task.map (Tuple.pair (List.map (Result.map (Maybe.map text)) reads)))
                        )
                    |> Task.map (Expect.equal ( [ Err Stream.TimedOut, Err Stream.TimedOut ], 1 ))
        , effect "a reader made by a task lasts as long as the fiber that made it" <|
            \_ ->
                Fiber.fork (chunks [ "a" ])
                    |> Task.andThen Fiber.join
                    |> Task.andThen (\from -> result (readText from))
                    |> Task.map (Expect.equal (Err Stream.Closed))
        , effect "fromBytes gives the bytes, then ends" <|
            \_ ->
                Stream.fromBytes (Nomi.Bytes.fromString "all")
                    |> Task.andThen (\from -> Task.map2 Tuple.pair (readText from) (readText from))
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal ( Just "all", Nothing ))
        ]



-- LINES


lines : Test
lines =
    describe "lines"
        [ effect "lines split across chunks, each newline \\n or \\r\\n" <|
            \_ ->
                chunks [ "a\nb", "c\u{000D}", "\n\nd\u{000D}e" ]
                    |> Task.andThen Stream.lines
                    |> Task.andThen allLines
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ "a", "bc", "", "d\u{000D}e" ])
        , effect "a line reader leaves the rest of the stream to the next line or read" <|
            \_ ->
                chunks [ "one\ntw", "o\nthree" ]
                    |> Task.andThen
                        (\from ->
                            Task.sequence
                                [ Stream.lines from |> Task.andThen Stream.line
                                , Stream.lines from |> Task.andThen Stream.line
                                , readText from
                                , readText from
                                ]
                        )
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ Just "one", Just "two", Just "three", Nothing ])
        , effect "lines of a task's reader are answered in order" <|
            \_ ->
                slowChunks 100 [ "a\nb", "\nc\n" ]
                    |> Task.andThen (\from -> concurrently (List.repeat 4 (Stdio.line from)))
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ Just "a", Just "b", Just "c", Nothing ])
        , effectWith (Nomi.Test.env |> Nomi.Test.withStdin "a\nb\nc\n") "lines of the kernel's reader are answered in order" <|
            \env ->
                concurrently (List.repeat 4 (Stdio.line env.stdin))
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ Just "a", Just "b", Just "c", Nothing ])
        , effect "a line that isn't UTF-8 fails with Invalid, and the next is read" <|
            \_ ->
                Stream.fromBytes (fromList [ 0x6F, 0x6B, 0x0A, 0xFF, 0x0A, 0x61 ])
                    |> Task.andThen Stream.lines
                    |> Task.andThen (\from -> Task.sequence (List.repeat 4 (result (Stream.line from))))
                    |> Task.map (Expect.equal [ Ok (Just "ok"), Err (Stream.Invalid "a line is not UTF-8"), Ok (Just "a"), Ok Nothing ])
        ]



-- WRITING


writing : Test
writing =
    describe "writing"
        [ effect "writes reach the writer in the order they came, each once the last is taken" <|
            \_ ->
                Ref.make []
                    |> Task.andThen
                        (\log ->
                            Stream.writer
                                { write = \bytes -> Process.sleep 1000 |> Task.andThen (\_ -> Time.now) |> Task.andThen (\now -> Ref.update ((::) ( text bytes, Time.posixToMillis now )) log)
                                , close = Task.succeed ()
                                }
                                |> Task.andThen (\to -> concurrently (List.map (\word -> Stdio.print word to) [ "a", "b", "c" ]))
                                |> Task.andThen (\_ -> Ref.get log)
                        )
                    |> Task.mapError Stream.errorToString
                    |> Task.map (List.reverse >> Expect.equal [ ( "a", 1000 ), ( "b", 2000 ), ( "c", 3000 ) ])
        , effect "a closed writer fails a write with Closed, and closes again quietly" <|
            \env ->
                Stdio.print "kept" env.stdout
                    |> Task.andThen (\_ -> Stream.close env.stdout)
                    |> Task.andThen (\_ -> Task.map2 Tuple.pair (result (Stdio.print "lost" env.stdout)) (result (Stream.close env.stdout)))
                    |> Task.mapError Stream.errorToString
                    |> Task.andThen (\after -> Nomi.Test.stdout env |> Task.map (Tuple.pair after))
                    |> Task.map (Expect.equal ( ( Err Stream.Closed, Ok () ), "kept" ))
        , effect "pipe writes the whole stream and leaves the writer open" <|
            \env ->
                chunks [ "x", "y" ]
                    |> Task.andThen (\from -> Stream.pipe from env.stdout)
                    |> Task.andThen (\_ -> Stdio.printLine "z" env.stdout)
                    |> Task.mapError Stream.errorToString
                    |> Task.andThen (\_ -> Nomi.Test.stdout env)
                    |> Task.map (Expect.equal "xyz\n")
        ]



-- STDIO


stdio : Test
stdio =
    describe "stdio"
        [ effectWith (Nomi.Test.env |> Nomi.Test.withStdin "one\u{000D}\ntwo") "the fake input reads by lines and by chunks" <|
            \env ->
                Task.sequence [ Stdio.line env.stdin, readText env.stdin, Stdio.line env.stdin ]
                    |> Task.mapError Stream.errorToString
                    |> Task.map (Expect.equal [ Just "one", Just "two", Nothing ])
        , effectWith (Nomi.Test.env |> Nomi.Test.withStdinBytes (fromList [ 0xC3, 0x28, 0x0A ])) "input that isn't UTF-8 fails Stdio.line with Invalid" <|
            \env ->
                result (Stdio.line env.stdin)
                    |> Task.map (Expect.equal (Err (Stream.Invalid "a line is not UTF-8")))
        , effect "a test's streams are no terminal" <|
            \env -> terminal env |> Task.map (Expect.equal ( ( False, False ), Nothing ))
        , effectWith (Nomi.Test.env |> Nomi.Test.withTerminal { columns = 80, rows = 24 }) "withTerminal makes them one" <|
            \env -> terminal env |> Task.map (Expect.equal ( ( True, True ), Just { columns = 80, rows = 24 } ))
        , effect "output that isn't UTF-8 reads as U+FFFD" <|
            \env ->
                Stream.write (fromList [ 0x61, 0xFF, 0x62 ]) env.stderr
                    |> Task.mapError Stream.errorToString
                    |> Task.andThen (\_ -> Nomi.Test.stderr env)
                    |> Task.map (Expect.equal "a\u{FFFD}b")
        ]


terminal : { a | stdin : Reader, stdout : Stream.Writer } -> Task x ( ( Bool, Bool ), Maybe { columns : Int, rows : Int } )
terminal env =
    Task.map3 (\interactive isTerminal size -> ( ( interactive, isTerminal ), size ))
        (Stdio.isInteractive env.stdin)
        (Stdio.isTerminal env.stdout)
        (Stdio.size env.stdout)



-- HELPERS


{-| A reader made by a task, of these chunks.
-}
chunks : List String -> Task x Reader
chunks =
    slowChunks 0


{-| A reader made by a task, of these chunks, each given this many
milliseconds after it is asked for.
-}
slowChunks : Int -> List String -> Task x Reader
slowChunks ms given =
    Ref.make given
        |> Task.andThen
            (\left ->
                Stream.reader
                    (Process.sleep (toFloat ms)
                        |> Task.andThen (\_ -> Ref.modify (\rest -> ( List.head rest, List.drop 1 rest )) left)
                        |> Task.map (Maybe.map Nomi.Bytes.fromString)
                    )
            )


{-| Each task on a fiber of its own, forked in order, and their results in
that order.
-}
concurrently : List (Task x a) -> Task x (List a)
concurrently tasks =
    tasks
        |> List.map Fiber.fork
        |> Task.sequence
        |> Task.andThen (List.map Fiber.join >> Task.sequence)


allLines : Stream.LineReader -> Task Stream.Error (List String)
allLines from =
    Stream.line from
        |> Task.andThen
            (\next ->
                case next of
                    Just line ->
                        allLines from |> Task.map ((::) line)

                    Nothing ->
                        Task.succeed []
            )


readText : Reader -> Task Stream.Error (Maybe String)
readText from =
    Stream.read from |> Task.map (Maybe.map text)


result : Task x a -> Task y (Result x a)
result task =
    task |> Task.map Ok |> Task.onError (Err >> Task.succeed)


text : Bytes -> String
text bytes =
    Nomi.Bytes.toString bytes |> Maybe.withDefault "<not UTF-8>"


fromList : List Int -> Bytes
fromList ints =
    Encode.encode (Encode.sequence (List.map Encode.unsignedInt8 ints))
