module Nomi.Path exposing
    ( join, normalize, parent, name, extension, relative
    , matches
    )

{-| Paths as text, `/`-separated, as `Nomi.Fs` takes them. Each function is
pure, and none asks the file system anything: `normalize "a/link/.."` is
`"a"` whatever `link` leads to.

A path is relative, as `"data/todos.json"`, or absolute, starting with
`/`. `.` is where a path starts, and `..` the directory above.

@docs join, normalize, parent, name, extension, relative


# Globs

@docs matches

-}

import Elm.Kernel.NomiPath


{-| The paths, one after another, normalized: `join [ "data", "../logs",
"today.txt" ]` is `"logs/today.txt"`, and `join []` is `"."`. A part
starting with `/` doesn't start over: `join [ "a", "/b" ]` is `"a/b"`.
-}
join : List String -> String
join parts =
    normalize (String.join "/" (List.filter (not << String.isEmpty) parts))


{-| The path with `.`, empty names and the `..` after a name taken out:
`"./a//b/../c/"` is `"a/c"`. A relative path keeps the `..`s that climb
above where it starts (`"../a"`), an absolute one drops them, and a path
that leads where it starts is `"."`.
-}
normalize : String -> String
normalize path =
    let
        absolute =
            String.startsWith "/" path

        step part names =
            case ( part, names ) of
                ( "", _ ) ->
                    names

                ( ".", _ ) ->
                    names

                ( "..", [] ) ->
                    if absolute then
                        []

                    else
                        [ ".." ]

                ( "..", ".." :: _ ) ->
                    ".." :: names

                ( "..", _ :: rest ) ->
                    rest

                _ ->
                    part :: names

        normal =
            String.join "/" (List.reverse (List.foldl step [] (String.split "/" path)))
    in
    if absolute then
        "/" ++ normal

    else if normal == "" then
        "."

    else
        normal


{-| The directory the path is in: `parent "a/b.txt"` is `"a"`, `parent "a"`
is `"."`, `parent "."` is `".."`, and `parent "/"` is `"/"`.
-}
parent : String -> String
parent path =
    normalize (path ++ "/..")


{-| The path's last name: `name "a/b.txt"` is `"b.txt"`. A path that names
nothing of its own, as `"."`, `".."` or `"/"`, has `""`.
-}
name : String -> String
name path =
    case List.reverse (String.split "/" (normalize path)) of
        last :: _ ->
            if last == "." || last == ".." then
                ""

            else
                last

        [] ->
            ""


{-| What follows the last `.` in the path's name: `extension "a.tar.gz"` is
`Just "gz"`. A name with no `.` but at its start, as `".bashrc"`, has
none.
-}
extension : String -> Maybe String
extension path =
    case List.reverse (String.indexes "." (name path)) of
        dot :: _ ->
            if dot == 0 then
                Nothing

            else
                Just (String.dropLeft (dot + 1) (name path))

        [] ->
            Nothing


{-| The path that leads from the directory `from` to `to`, both relative to
one place or both absolute: `relative "a/b" "a/c/d"` is `"../c/d"`.
`Nothing` when no path does without knowing more: `from` relative and
`to` absolute or the other way round, or `from` climbing above what the
two share, as `relative ".." "a"`.
-}
relative : String -> String -> Maybe String
relative from to =
    let
        parts path =
            String.split "/" (normalize path) |> List.filter (\part -> part /= "" && part /= ".")

        drop a b =
            case ( a, b ) of
                ( x :: xs, y :: ys ) ->
                    if x == y then
                        drop xs ys

                    else
                        ( a, b )

                _ ->
                    ( a, b )

        ( up, down ) =
            drop (parts from) (parts to)
    in
    if String.startsWith "/" from /= String.startsWith "/" to || List.member ".." up then
        Nothing

    else
        Just (join (List.map (\_ -> "..") up ++ down))


{-| Whether the path matches the glob: `*` any characters but `/`, `**` any
directories, `?` one character, `[abc]` and `[a-z]` one of them, `[!abc]`
one not among them, and `{a,b}` either. A path matches whole, as
`Bun.Glob` matches it.

    matches "src/**/*.elm" "src/Page/Home.elm" -- True

-}
matches : String -> String -> Bool
matches =
    Elm.Kernel.NomiPath.matches
