module FsTest exposing (suite)

import Expect
import Nomi.Bytes
import Nomi.Fiber as Fiber
import Nomi.Fs as Fs exposing (Fs)
import Nomi.Queue as Queue
import Nomi.Stream as Stream
import Nomi.Test exposing (effect, effectWith)
import Nomi.Task
import Process
import Task exposing (Task)
import Test exposing (Test, describe)
import Time


suite : Test
suite =
    describe "Nomi.Fs in memory"
        [ watching
        , effect "a file's time is the test clock's when it was written" <|
            \env ->
                Fs.write "a.txt" "a" env.fs
                    |> Task.andThen (\_ -> Process.sleep 5000)
                    |> Task.andThen (\_ -> Fs.write "b.txt" "b" env.fs)
                    |> Task.andThen (\_ -> Task.map2 Tuple.pair (Fs.stat "a.txt" env.fs) (Fs.stat "b.txt" env.fs))
                    |> Task.map (Tuple.mapBoth (.modified >> Time.posixToMillis) (.modified >> Time.posixToMillis))
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal ( 0, 5000 ))
        , effectWith (Nomi.Test.env |> Nomi.Test.withFiles [ ( "data/big.csv", "a,b\n1,2\n" ) ]) "a file read as a stream, by lines" <|
            \env ->
                Fs.reader "data/big.csv" env.fs
                    |> Task.mapError Fs.errorToString
                    |> Task.andThen (\from -> Stream.lines from |> Task.andThen (\lines -> Task.sequence (List.repeat 3 (Stream.line lines))) |> Task.mapError Stream.errorToString)
                    |> Task.map (Expect.equal [ Just "a,b", Just "1,2", Nothing ])
        , effect "a file written as a stream holds each write, once its writer is closed or not" <|
            \env ->
                Fs.writer "out.txt" env.fs
                    |> Task.mapError Fs.errorToString
                    |> Task.andThen
                        (\to ->
                            Stream.write (Nomi.Bytes.fromString "one ") to
                                |> Task.andThen (\_ -> Stream.write (Nomi.Bytes.fromString "two") to)
                                |> Task.mapError Stream.errorToString
                        )
                    |> Task.andThen (\_ -> Fs.read "out.txt" env.fs |> Task.mapError Fs.errorToString)
                    |> Task.map (Expect.equal "one two")
        , effect "a temporary directory is a file system of its own" <|
            \env ->
                Fs.temporaryDirectory env.fs
                    |> Task.andThen (\scratch -> Fs.write "t.txt" "t" scratch |> Task.andThen (\_ -> Task.map2 Tuple.pair (Fs.list "." scratch) (Fs.list "." env.fs)))
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal ( [ "t.txt" ], [] ))
        ]


watching : Test
watching =
    describe "watching"
        [ effectWith (Nomi.Test.env |> Nomi.Test.withFiles [ ( "src/Main.elm", "" ) ]) "changes gather until a change takes them, each path once" <|
            \env ->
                Fs.watch "src" env.fs
                    |> Task.andThen
                        (\watch ->
                            Fs.write "src/Main.elm" "a" env.fs
                                |> Task.andThen (\_ -> Fs.write "src/Main.elm" "b" env.fs)
                                |> Task.andThen (\_ -> Fs.makeDirectory "src/Page" env.fs)
                                |> Task.andThen (\_ -> Fs.write "src/Page/Home.elm" "" env.fs)
                                |> Task.andThen (\_ -> Fs.write "elsewhere.txt" "" env.fs)
                                |> Task.andThen (\_ -> Fs.change watch)
                        )
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal (Just { paths = [ "Main.elm", "Page", "Page/Home.elm" ] }))
        , effect "a change waits for one" <|
            \env ->
                Fs.watch "." env.fs
                    |> Task.andThen
                        (\watch ->
                            Fiber.fork (Process.sleep 1000 |> Task.andThen (\_ -> Fs.write "later.txt" "" env.fs))
                                |> Task.andThen (\_ -> Fs.change watch)
                        )
                    |> Task.andThen (\changed -> Time.now |> Task.map (Tuple.pair changed))
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal ( Just { paths = [ "later.txt" ] }, Time.millisToPosix 1000 ))
        , effect "past a thousand paths, a change says only that something did" <|
            \env ->
                Fs.watch "." env.fs
                    |> Task.andThen
                        (\watch ->
                            List.range 1 1001
                                |> List.map (\i -> Fs.write (String.fromInt i ++ ".txt") "" env.fs)
                                |> Task.sequence
                                |> Task.andThen (\_ -> Fs.change watch)
                        )
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal (Just { paths = [ "." ] }))
        , effect "a watch stops once the fiber that made it ends" <|
            \env ->
                Queue.unbounded
                    |> Task.andThen
                        (\watches ->
                            Fiber.fork (Fs.watch "." env.fs |> Task.andThen (\watch -> Queue.offer watch watches) |> Task.andThen (\_ -> Process.sleep 1000))
                                |> Task.andThen (\_ -> Queue.take watches)
                        )
                    |> Task.andThen (\watch -> Task.map2 Tuple.pair (Fs.change watch) Time.now)
                    |> Task.mapError Fs.errorToString
                    |> Task.map (Expect.equal ( Nothing, Time.millisToPosix 1000 ))
        , effect "a watch of what isn't there fails" <|
            \env ->
                Fs.watch "missing" env.fs
                    |> Task.map (\_ -> ())
                    |> Task.map Ok
                    |> Task.onError (Err >> Task.succeed)
                    |> Task.map (Expect.equal (Err (Fs.NotFound "missing")))
        ]
