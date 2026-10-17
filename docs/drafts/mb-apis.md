# Backend platform (MB.2)

Status: agreed with the maintainer, 2026-10-17, and refined the same day after a review against the code and Bun 1.4.3 (`plan: refine MB`). Its decisions are in architecture.md §8, and plan MB's steps are written from §6 below. The survey behind it (what Bun, Deno, Go, Gleam, Node, Effect, ZIO and Roc ship) and the spike that ran nomi on Bun are kept with the development notes (`~/.cache/nomi-dev/mb/survey.md`, `~/.cache/nomi-dev/bun/`).

## 1. Principles

1. **The Elm way.** No FFI. Every effect is a function of a `nomi/*` package, typed, documented and tested; only nomi's bundled packages carry kernel code. Everything else, the community's packages included, is pure Elm built on them.
2. **Blocks first, batteries on top.** nomi ships the low-level blocks an ecosystem needs (bytes, streams, processes, files, sockets, TLS, crypto) and the batteries a backend reaches for every day (an HTTP server with WebSockets, SQL, Redis, S3). A battery is written on the same kind of API a community package would use.
3. **Bun is the host.** nomi's server programs run on Bun only, and the kernels use Bun's built-ins (`Bun.serve`, `Bun.sql`, `Bun.redis`, `Bun.s3`, `Bun.spawn`, `Bun.password`) and its Node-compatible modules, never npm. Where Bun's built-in falls short of a guarantee below, the kernel makes up for it.
4. **Authority is a value.** What can touch the world beyond the program (files, network, processes) is a capability handed to `main` in `Env`, narrowed by ordinary functions, and absent unless passed. A package can only touch what it is handed, but for the elm/* effects (`Http` above all), which stay ambient so that code shared with the browser keeps working.
5. **Every capability has a fake.** `nomi/test` runs any program's tasks on fakes, as it already runs `Fs` in memory and time on a virtual clock.
6. **Interruption cleans up.** A fiber interrupted while it holds a resource releases it: a socket closes, a child is killed, a query is cancelled, a transaction rolls back, an upload is aborted. Each module documents this and tests it, and says what it does where the host can't (D9).

## 2. Decisions

**D1. Bun only.** Programs built for the server run on Bun 1.4 or later: `nomi run` and `nomi test` start `bun`, a built entry is an ES module for Bun, and the runtime's probe refuses an older Bun or another host with a message. Node is no longer a target. Plain Elm programs keep building for the browser exactly as elm builds them, and their tests run on Bun too. Supporting Node beside Bun would mean two kernels for every module, each tested on both, with Node reimplementing what Bun gives (a WebSocket server, SQL, Redis, S3, argon2).

**D2. Bytes are the unit of I/O.** Every reader, writer, socket, file, body, pipe, hash and blob is `Bytes`, elm/bytes' type, so `Bytes.Encode` and `Bytes.Decode` work on everything. Text is a layer at the edges: `Fs.read`, `Stdio.printLine` and `Request.text` stay, and decode or encode UTF-8 strictly, failing with an error rather than replacing bad bytes. Paths, headers, arguments, environment variables and SQL text stay `String`. `Nomi.Bytes` adds what elm/bytes lacks, as pure functions on kernel code.

**D3. Streams are pull handles.** One pair of opaque types in nomi/core that every source and sink gives:

```elm
read : Reader -> Task Error (Maybe Bytes)     -- the next chunk, Nothing at the end
write : Bytes -> Writer -> Task Error ()      -- succeeds once the sink accepted it
close : Writer -> Task Error ()
```

A write that waits for the sink is the backpressure. An interrupted `read` loses nothing. A handle holds state (a position, a buffer, a decompressor), so whatever makes one is a task, as `Ref.make` is: transforms are `Reader -> Task x Reader` (`Compress.gunzip`, `Stream.limit`), and text lines are a `LineReader` (`Stream.lines : Reader -> Task x LineReader`, `Stream.line : LineReader -> Task Error (Maybe String)`), clearer to use than lines as chunks. Made by a pure function, a handle would be two where a `let` makes one, and `Stream.line (Stream.lines reader)` in a loop would drop what each chunk held past its line. Elm code makes handles of its own from tasks (`Stream.reader`, `Stream.writer`), so a transform, a protocol or a multipart part is pure Elm on the same API as the kernel's. A combinator library (`fold`, `map`, `pipe`, `take`) sits on top with no kernel. On Bun, a body is a `ReadableStream` of the default kind, which waits for a slow reader; the `direct` kind buffers whatever is written, so kernels don't use it.

`Stream.Error` is one type for every source and sink: the other end gone (`Closed`), a limit passed (`TooLarge`), data a reader refuses (`Invalid`: bad UTF-8, a corrupt gzip), and the system's failure by kind (`ConnectionReset`, `PermissionDenied`, `TimedOut` and the like), with `errorToString`. Opening a stream fails with its module's error; once open, it fails with `Stream.Error`.

**D4. What `Env` carries.**

| Kind | What | Why |
|---|---|---|
| Capability, in `Env` | `fs`, `net` (connect and listen, one capability, narrowed with `Net.onlyHosts` and `Net.noListen`), `processes`, `stdin`, `stdout`, `stderr` | authority over the world |
| Derived capability | a database, a Redis client, an S3 bucket | made from `net` or `fs` and a config |
| Data, in `Env` | `args`, `vars` | snapshots; `Sql.configFromEnv env.vars` reads them explicitly |
| Ambient task | clock, sleep, secure randomness, logging through a `Logger` value | no authority; each one controlled by the test runtime |
| Pure function | hashes, HMAC, constant-time compare, encodings, presigning at a given time, compression of small values | deterministic, so no task |

**D5. Fakes.** Capabilities are opaque kernel values, and each module that makes one makes its fake too, so a fake has exactly the real API: `Fs.memory`, `Net.loopback` (an in-memory network on which a program's own server answers its clients), `Process.fake`, `S3.fromFs`, `Redis.memory`. A fake holds state, so making one is a task, and it is inspected through its own module (a memory `Fs`'s files through `Fs`, what `Process.fake` ran through `Process.ran`), so nomi/test depends on no battery. SQLite in memory is the real thing; Postgres, MySQL, Redis and S3 also run for real, in containers (§6).

**D6. Resources belong to the fiber that opens them.** A handle (a file, a child, a socket, a pool, a watch) is released when the fiber that opened it ends, however it ends, on the runtime's scopes; `with` functions (`Fs.withFile`) narrow one to a task, and an explicit `close` exists where a program closes early. A handle fibers share, such as a pool or a client, is opened by a fiber that outlives them, usually `main`'s.

An event source (`Fs.watch`, `Process.signals`, `Sql.listen`, a Redis subscription) is such a handle, read with a task as a `Reader` is: the next event, `Nothing` once the source ends, and its module's error when it breaks, as when a LISTEN connection is lost. Its buffer is bounded: file changes and signals coalesce, and a listener or subscription that falls past the bound fails with an overflow error. A bare `Queue` would never end, never fail, and grow.

**D7. Errors are per module and typed.** Each module has its own error type with `errorToString`, and failures are constructors (`NotFound`, `PermissionDenied`, `ConnectionRefused`, `TimedOut`, a database's SQLSTATE), never strings to match on. Streams share one, `Stream.Error` (D3).

**D8. Packages.** All bundled with nomi, one version of each:

| Package | Modules |
|---|---|
| `nomi/core` | `Nomi`, `Nomi.Task`, `Nomi.Fiber`, `Nomi.Ref`, `Nomi.Queue`, `Nomi.Bytes`, `Nomi.Stream`, `Nomi.Fs`, `Nomi.Path`, `Nomi.Stdio`, `Nomi.Process`, `Nomi.Net`, `Nomi.Crypto`, `Nomi.Random`, `Nomi.Password`, `Nomi.Log`, `Nomi.Compress` |
| `nomi/net` | `Nomi.Tcp`, `Nomi.Tls`, `Nomi.Udp`, `Nomi.Dns` |
| `nomi/http` | `Nomi.Http.Server` (today's `Nomi.Server`), `Nomi.Http.Client`, `Nomi.Http.WebSocket`, and the pure `Nomi.Http.Route`, `Nomi.Http.Cookie`, `Nomi.Http.Form` |
| `nomi/sql` | `Nomi.Sql` |
| `nomi/redis` | `Nomi.Redis` |
| `nomi/s3` | `Nomi.S3` |
| `nomi/test` | as today, on nomi/core's `Env` |

`Nomi.Net`, the capability with its narrowing and `loopback`, is in nomi/core because `Env` carries a `Net`, and nomi/net's modules build on nomi/core: the other way round, the two packages would depend on each other. `nomi/server` becomes `nomi/http`. A package whose API changes takes a new major version, as elm.json's rules require: nomi/core (`Stdio`, `Env`), nomi/test (whose `Env` is nomi/core's) and nomi/http are 2.0.0 in 0.7, and nomi/net, nomi/sql, nomi/redis and nomi/s3 start at 1.0.0. A project naming `nomi/server` or a 1.x range is told what nomi bundles. The batteries are marked experimental in their docs until a release has run them in production.

**D9. One SQL driver.** `Nomi.Sql` is one module over `Bun.sql`, which speaks Postgres, MySQL and SQLite; `bun:sqlite` isn't used, so there is one SQLite in nomi, not two. What that gives up (loading extensions, custom SQL functions, serializing a database to bytes) is rarely needed by a server and can come later through `Bun.sql` if it grows them.

- **64-bit integers.** SQLite opens with `safeIntegers`, and an Elm `Int` is exact only to ±(2^53 − 1), so `Sql.int` fails past that rather than round, `Sql.int64` reads any integer column exactly as its decimal text, and a parameter past it is passed as `Text`.
- **SQLite runs on the event loop.** `Bun.sql` runs a SQLite statement synchronously: on Bun 1.4.3 a 400 ms query let no timer fire, and its `cancel` couldn't run until it ended. So every fiber waits while a SQLite statement runs, and a statement whose fiber is interrupted runs to its end, the interruption taking effect once it returns. The docs say so; moving SQLite off the event loop waits for workers (D11). Postgres and MySQL queries are cancelled on the server (§4).

**D10. Routing, cookies and forms ship, as pure Elm,** small and replaceable, in `nomi/http`. Sessions, CSRF, JWT and templating are left to packages.

**D11. Workers are after 0.7.** Fibers give concurrency on one thread; workers running another nomi module, for CPU-bound work and for SQLite, are refined once 0.7 ships.

## 3. Foundation

```elm
-- Nomi.Bytes: pure, kernel-backed, on elm/bytes' Bytes
length : Bytes -> Int
slice : Int -> Int -> Bytes -> Bytes
append : Bytes -> Bytes -> Bytes
concat : List Bytes -> Bytes
equal : Bytes -> Bytes -> Bool
indexOf : Bytes -> Bytes -> Maybe Int
fromString : String -> Bytes                 -- UTF-8
toString : Bytes -> Maybe String             -- strict UTF-8
toBase64, toBase64Url, toHex : Bytes -> String
fromBase64, fromHex : String -> Maybe Bytes

-- Nomi.Stream
type Reader
type Writer
type LineReader
type Error
read : Reader -> Task Error (Maybe Bytes)
readAll : { limit : Int } -> Reader -> Task Error Bytes
write : Bytes -> Writer -> Task Error ()
close : Writer -> Task Error ()
pipe : Reader -> Writer -> Task Error ()
reader : Task Error (Maybe Bytes) -> Task x Reader       -- each read runs the task
writer : { write : Bytes -> Task Error (), close : Task Error () } -> Task x Writer
fromBytes : Bytes -> Task x Reader
limit : Int -> Reader -> Task x Reader
lines : Reader -> Task x LineReader
line : LineReader -> Task Error (Maybe String)
```

- `Nomi.Stdio`: `stdin` is a `Reader`, `stdout` and `stderr` are `Writer`s; `line`, `print` and `printLine` stay as text helpers, `line` failing with `Invalid` on bad UTF-8. Adds `isTerminal` and `size`.
- `Nomi.Process`: `exit` stays; adds `cwd`, `pid`, `platform`, and `signals : List Signal -> Task x Signals`, a handle (D6) read with `signal : Signals -> Task x Signal`. SIGINT and SIGTERM end the program whether or not it listens for them.
- The runtime formats a defect itself (its name, message and stack), so a report reads the same whatever the host prints.

## 4. Tier 1

**Files (`Nomi.Fs`, `Nomi.Path`).** Adds `stat`, `rename`, `copy` (recursive), `remove` with `{ recursive }`, `makeDirectory` with parents, `symlink`, `readLink`, `realPath`, `temporaryDirectory`, `withFile` opening a `Reader` or `Writer`, and `watch : String -> Fs -> Task Error Watch`, a handle (D6) read with `change : Watch -> Task Error (Maybe Change)`, its events batched and documented as hints. `Fs.read` decodes UTF-8 strictly. `Nomi.Path` is pure: `join`, `parent`, `name`, `extension`, `normalize`, `relative`, glob matching. `subdir` resolves symlinks before allowing a path. `Fs.memory` fakes it all.

**Processes (`Nomi.Process`).** On `Bun.spawn`, its argument list never a shell string:

```elm
command : String -> List String -> Command
withCwd : String -> Command -> Command
withEnv : Dict String String -> Command -> Command
withStdin : Bytes -> Command -> Command
output : Command -> Processes -> Task Error { status : Int, stdout : Bytes, stderr : Bytes }
spawn : Command -> Processes -> Task Error Child   -- stdin : Writer, stdout/stderr : Reader
wait : Child -> Task Error Int
kill : Signal -> Child -> Task Error ()
fake : (Command -> Maybe { status : Int, stdout : Bytes, stderr : Bytes }) -> Task x Processes
ran : Processes -> Task x (List Command)
```

A child belongs to the fiber that spawned it (D6): interrupting the fiber sends SIGTERM, then SIGKILL after a grace. Windows has no SIGTERM, so there `kill` and interruption end a child at once. `fake` answers `output` and `spawn` from its function and refuses a command the function doesn't know; `ran` lists the commands a fake ran.

**HTTP server (`Nomi.Http.Server`).** On `Bun.serve`. The handler stays `Request -> Task Never Response`, its fiber interrupted when the client goes away (Bun aborts the request's signal). `body : Request -> Reader`, with `bytes`, `text`, `json` and `form` helpers that take a limit. A response is text, JSON, bytes, a `Reader` (SSE with the right content type) or a file. `upgrade : (WebSocket -> Task Never ()) -> Response` makes the connection a fiber that receives and sends messages, `send` waiting while Bun reports backpressure until `drain`. TLS from a certificate and key; graceful shutdown as now. Pure helpers: `Route`, `Cookie` (parse, serialize, sign with `Nomi.Crypto`'s HMAC), `Form` (urlencoded, and multipart streaming a `Reader` per part, made with `Stream.reader`).

**HTTP client (`Nomi.Http.Client`).** Building a request is pure; `send : Request -> Net -> Task Error (Response Reader)` on `fetch`, a status being data, not an error. The client follows redirects itself, so `Net.onlyHosts` holds for each hop. elm/http keeps working, on `fetch`, for code shared with the browser. `Net.loopback` routes a client's requests to a handler.

**Crypto (`Nomi.Crypto`, `Nomi.Random`, `Nomi.Password`).** Pure: `hash : Algorithm -> Bytes -> Bytes` (SHA-1, SHA-2, SHA-3, BLAKE2), `hmac`, `equalConstantTime`; `hashReader` streaming. Tasks: `Random.bytes`, `Random.uuid`, `Random.uuidV7`, seeded per test under `nomi test`. `Password.hash` and `Password.verify` on `Bun.password` (argon2id, verifying bcrypt too). Keys, signatures and encryption are tier 2.

**Compression (`Nomi.Compress`).** gzip, deflate, brotli and zstd, pure for small values and `Reader -> Task x Reader` for streams (D3).

**Logging (`Nomi.Log`).** A `Logger` is a value passed explicitly, as Elm passes dependencies, rather than annotations fibers carry, since architecture §8 takes no FiberRefs: `logger : Logger` writes to stderr, `with : List ( String, Value ) -> Logger -> Logger` adds fields to every line it writes, and `info : String -> List ( String, Value ) -> Logger -> Task x ()` and the other levels write one. `Value` is the module's own (`string`, `int`, `float`, `bool`), so nomi/core needs no elm/json. Text on a terminal, JSON lines otherwise; under `nomi test` the lines go to the test's stderr. `Nomi.log` stays.

**SQL (`Nomi.Sql`).** One module, three databases:

```elm
type Value = Null | Int Int | Float Float | Text String | Blob Bytes | Bool Bool
connect : Config -> Net -> Task Error Db               -- Postgres or MySQL, a pool
open : String -> Fs -> Task Error Db                    -- SQLite, a file under the capability's root
memory : Task x Db                                      -- SQLite in memory
query : String -> List Value -> Decoder row -> Db -> Task Error (List row)
execute : String -> List Value -> Db -> Task Error { changes : Int }
transaction : (Error -> e) -> (Db -> Task e a) -> Db -> Task e a   -- rolls back on any failure or interruption
listen : String -> Db -> Task Error Listener            -- Postgres LISTEN, a handle (D6)
notification : Listener -> Task Error (Maybe String)
int : Decoder Int                                       -- fails past ±(2^53 − 1)
int64 : Decoder String                                  -- any integer, exactly, as decimal text
```

Row decoders read columns by name, as `Json.Decode` reads fields. `transaction` takes how a database error becomes the program's, so the task inside fails with the program's own errors and any failure rolls back. A Postgres or MySQL query whose fiber is interrupted is cancelled: `Bun.sql` can't cancel one already running, so the kernel runs each on a reserved connection, knows its backend, and cancels it from a connection kept outside the pool, which an exhausted pool can't hold up (`pg_cancel_backend` on Postgres, `KILL QUERY` on MySQL). A SQLite statement runs to its end (D9). `open` refuses a path outside its capability's root, opens read-only under `Fs.readOnly`, and fails with `NotOnDisk` on an `Fs` with no disk behind it, such as `Fs.memory`; tests use `memory`. Results arrive whole, as `Bun.sql` has no cursors; `query` takes no limit of its own, and a large result is paged in SQL.

## 5. Tier 2 and the batteries

**Blocks.** `Nomi.Tcp` (connect, and listen with a fiber per connection) on `Bun.connect` and `Bun.listen`, `Nomi.Tls` (connect, listen, upgrading a connection), Unix sockets where the OS has them, `Nomi.Udp` on `Bun.udpSocket`, `Nomi.Dns`, a WebSocket client; each on `Net.loopback` too. With these a community package can write a driver for what nomi doesn't ship, in pure Elm.

**Batteries.**
- **Redis** on `Bun.redis`: the common commands typed, `command : List Bytes -> Redis -> Task Error Reply` for the rest, pub/sub as a handle a fiber reads (D6). A client multiplexes one connection, so blocking commands and subscriptions get a connection of their own (`duplicate`), and interruption closes that one, cancelling the command without failing other fibers' commands.
- **S3** on `Bun.s3`: get and put as streams, head, delete, list, presigned URLs; tested against Garage in a container, since MinIO stopped publishing images in October 2025 and has been archived since.

## 6. Order of work

1. **MB.3** The runtime the same on any host, still on Node: defects formatted by the runtime, test workers ending with their supervisor.
2. **MB.4** Bun as the host (D1); the platform named `bun`.
3. **MB.5** `Nomi.Bytes`.
4. **MB.6** `Nomi.Stream`; `Stdio` on it; nomi/core and nomi/test 2.0.0.
5. **MB.7** Files and `Nomi.Path`.
6. **MB.8** Processes and signals.
7. **MB.9** `nomi/http`: the server on streams, files, SSE and TLS.
8. **MB.10** The client, and `Nomi.Net`'s narrowing and loopback.
9. **MB.11** WebSockets.
10. **MB.12** Crypto, random, passwords and compression.
11. **MB.13** Routes, cookies and forms, on MB.12's HMAC.
12. **MB.14** Logging.
13. **MB.15** The services in containers, pinned by digest: a tier-2 suite, run on Linux.
14. **MB.16** `nomi/sql` on SQLite.
15. **MB.17** Postgres and MySQL.
16. **MB.18** `nomi/net`.
17. **MB.19** `nomi/redis`.
18. **MB.20** `nomi/s3`.
19. **MB.21** Examples on the platform, the README, and an exit review.
20. **MB.22** Release 0.7.
