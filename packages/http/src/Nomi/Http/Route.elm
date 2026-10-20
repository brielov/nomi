module Nomi.Http.Route exposing
    ( Route, parse
    , top, s, string, int, custom, rest
    , slash, map, oneOf
    )

{-| Routes: what a request's path, as `Nomi.Http.Server.path` gives it,
means, read into a type of the program's.

    type Page
        = Home
        | Todo Int
        | EditTodo Int

    page : Route (Page -> a) a
    page =
        Route.oneOf
            [ Route.map Home Route.top
            , Route.map Todo (Route.s "todos" |> Route.slash Route.int)
            , Route.map EditTodo (Route.s "todos" |> Route.slash Route.int |> Route.slash (Route.s "edit"))
            ]

    Route.parse page (Nomi.Http.Server.path req) -- Just (Todo 3) for /todos/3

@docs Route, parse


# Segments

@docs top, s, string, int, custom, rest


# Combining

@docs slash, map, oneOf

-}


{-| A route that reads path segments into a value: `a` is what the route
is given, a function taking what it reads, and `b` what it gives once
read.
-}
type Route a b
    = Route (State a -> List (State b))


type alias State value =
    { unread : List String
    , value : value
    }


{-| What the path means, once a route read every segment of it, or
`Nothing`. When more than one could, the first in `oneOf` wins.
-}
parse : Route (a -> a) a -> List String -> Maybe a
parse (Route route) segments =
    route { unread = segments, value = identity }
        |> List.filter (\state -> List.isEmpty state.unread)
        |> List.head
        |> Maybe.map .value



-- SEGMENTS


{-| No segment: the root, `/`, alone or after others.
-}
top : Route a a
top =
    Route (\state -> [ state ])


{-| A segment that must be this text.
-}
s : String -> Route a a
s expected =
    Route
        (\state ->
            case state.unread of
                next :: unread ->
                    if next == expected then
                        [ { unread = unread, value = state.value } ]

                    else
                        []

                [] ->
                    []
        )


{-| Any segment, as text.
-}
string : Route (String -> a) a
string =
    custom Just


{-| A segment that is an integer: `3`, or `-3`.
-}
int : Route (Int -> a) a
int =
    custom String.toInt


{-| A segment the function reads, or refuses with `Nothing`.

    slug : Route (String -> a) a
    slug =
        Route.custom (\segment -> if String.all (\c -> Char.isAlphaNum c || c == '-') segment then Just segment else Nothing)

-}
custom : (String -> Maybe value) -> Route (value -> a) a
custom read =
    Route
        (\state ->
            case state.unread of
                next :: unread ->
                    case read next of
                        Just value ->
                            [ { unread = unread, value = state.value value } ]

                        Nothing ->
                            []

                [] ->
                    []
        )


{-| Every segment left, however many, as for a file's path under a
directory served.
-}
rest : Route (List String -> a) a
rest =
    Route (\state -> [ { unread = [], value = state.value state.unread } ])



-- COMBINING


{-| The first route, then the second, as a path's segments follow each
other: `Route.s "todos" |> Route.slash Route.int` reads `/todos/3`.
-}
slash : Route b c -> Route a b -> Route a c
slash (Route after) (Route before) =
    Route (\state -> List.concatMap after (before state))


{-| What the route reads, given to the function, as `Route.map Todo`
gives a page from an id.
-}
map : a -> Route a b -> Route (b -> c) c
map value (Route route) =
    Route
        (\state ->
            route { unread = state.unread, value = value }
                |> List.map (\read -> { unread = read.unread, value = state.value read.value })
        )


{-| Any of the routes; the first that reads the whole path wins.
-}
oneOf : List (Route a b) -> Route a b
oneOf routes =
    Route (\state -> List.concatMap (\(Route route) -> route state) routes)
