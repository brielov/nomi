module PathTest exposing (suite)

import Expect
import Fuzz exposing (Fuzzer)
import Nomi.Path as Path
import Test exposing (Test, describe, fuzz, fuzz2, test)


suite : Test
suite =
    describe "Nomi.Path"
        [ table "normalize"
            Path.normalize
            [ ( "a/b", "a/b" )
            , ( "./a//b/../c/", "a/c" )
            , ( "", "." )
            , ( ".", "." )
            , ( "a/..", "." )
            , ( "../a", "../a" )
            , ( "../../a/..", "../.." )
            , ( "a/../../b", "../b" )
            , ( "/", "/" )
            , ( "/../a", "/a" )
            , ( "//a//b//", "/a/b" )
            ]
        , table "parent"
            Path.parent
            [ ( "a/b.txt", "a" )
            , ( "a", "." )
            , ( ".", ".." )
            , ( "..", "../.." )
            , ( "/", "/" )
            , ( "/a", "/" )
            , ( "a/b/", "a" )
            ]
        , table "name"
            Path.name
            [ ( "a/b.txt", "b.txt" )
            , ( "b.txt", "b.txt" )
            , ( "a/b/", "b" )
            , ( ".", "" )
            , ( "..", "" )
            , ( "/", "" )
            ]
        , table "extension"
            Path.extension
            [ ( "a.tar.gz", Just "gz" )
            , ( "dir.d/file", Nothing )
            , ( ".bashrc", Nothing )
            , ( "a.", Just "" )
            , ( "a", Nothing )
            , ( "a/.config.json", Just "json" )
            ]
        , table "join"
            Path.join
            [ ( [ "data", "../logs", "today.txt" ], "logs/today.txt" )
            , ( [], "." )
            , ( [ "a", "/b" ], "a/b" )
            , ( [ "/a", "b" ], "/a/b" )
            , ( [ "", "a", "" ], "a" )
            ]
        , table "relative"
            (\( from, to ) -> Path.relative from to)
            [ ( ( "a/b", "a/c/d" ), Just "../c/d" )
            , ( ( "a", "a" ), Just "." )
            , ( ( ".", "a/b" ), Just "a/b" )
            , ( ( "a/b", "." ), Just "../.." )
            , ( ( "../a", "../b" ), Just "../b" )
            , ( ( "/x/y", "/x/z" ), Just "../z" )
            , ( ( "..", "a" ), Nothing )
            , ( ( "/a", "a" ), Nothing )
            ]
        , table "matches"
            (\( glob, matched ) -> Path.matches glob matched)
            [ ( ( "src/**/*.elm", "src/Page/Home.elm" ), True )
            , ( ( "src/**/*.elm", "src/Main.elm" ), True )
            , ( ( "src/**/*.elm", "tests/Main.elm" ), False )
            , ( ( "*.{js,mjs}", "a.mjs" ), True )
            , ( ( "a/*", "a/b/c" ), False )
            , ( ( "?.txt", "a.txt" ), True )
            , ( ( "?.txt", "ab.txt" ), False )
            , ( ( "[!a]*", "bcd" ), True )
            , ( ( "[a-c].md", "b.md" ), True )
            ]
        , fuzz path "normalize is idempotent" <|
            \p -> Path.normalize (Path.normalize p) |> Expect.equal (Path.normalize p)
        , fuzz path "a path is its parent joined with its name" <|
            \p ->
                if Path.name p == "" then
                    Expect.pass

                else
                    Path.join [ Path.parent p, Path.name p ] |> Expect.equal (Path.normalize p)
        , fuzz2 relativePath relativePath "relative leads from one path to the other" <|
            \from to ->
                case Path.relative from to of
                    Just way ->
                        Path.join [ from, way ] |> Expect.equal (Path.normalize to)

                    Nothing ->
                        Expect.pass
        ]


table : String -> (a -> b) -> List ( a, b ) -> Test
table name f cases =
    test name <|
        \_ ->
            List.map (Tuple.first >> f) cases |> Expect.equal (List.map Tuple.second cases)


{-| Paths of a few names, `.`, `..` and empty names among them.
-}
path : Fuzzer String
path =
    Fuzz.map2 (++)
        (Fuzz.oneOf [ Fuzz.constant "", Fuzz.constant "/" ])
        relativePath


relativePath : Fuzzer String
relativePath =
    Fuzz.listOfLengthBetween 0 6 (Fuzz.oneOfValues [ "a", "b", "c.txt", ".", "..", "" ])
        |> Fuzz.map (String.join "/")
