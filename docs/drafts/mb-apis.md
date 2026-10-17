# Backend platform (MB.2)

Status: agreed with the maintainer, 2026-10-17. Its decisions are in architecture.md §8, and plan MB's steps are written from §6 below. The survey behind it (what Bun, Deno, Go, Gleam, Node, Effect, ZIO and Roc ship) and the spike that ran nomi on Bun are kept with the development notes (`~/.cache/nomi-dev/mb/survey.md`, `~/.cache/nomi-dev/bun/`).

## 1. Principles

1. **The Elm way.** No FFI. Every effect is a function of a `nomi/*` package, typed, documented and tested; only nomi's bundled packages carry kernel code. Everything else, the community's packages included, is pure Elm built on them.
2. **Blocks first, batteries on top.** nomi ships the low-level blocks an ecosystem needs (bytes, streams, processes, files, sockets, TLS, crypto) and the batteries a backend reaches for every day (an HTTP server with WebSockets, SQL, Redis, S3). A battery is written on the same kind of API a community package would use.
3. **Bun is the host.** nomi's server programs run on Bun only, and the kernels use Bun's built-ins (`Bun.serve`, `Bun.sql`, `Bun.redis`, `Bun.s3`, `Bun.spawn`, `Bun.password`) and its Node-compatible modules, never npm. Where Bun's built-in falls short of a guarantee below, the kernel makes up for it.
4. **Authority is a value.** What can touch the world beyond the program (files, network, processes) is a capability handed to `main` in `Env`, narrowed by ordinary functions, and absent unless passed. A package can only touch what it is handed.
5. **Every capability has a fake.** `nomi/test` runs any program's tasks on fakes, as it already runs `Fs` in memory and time on a virtual clock.
6. **Interruption cleans up.** A fiber interrupted while it holds a resource releases it: a socket closes, a child is killed, a query is cancelled, a transaction rolls back, an upload is aborted. Each module documents this and tests it.

## 2. Decisions

**D1. Bun only.** Programs built for the server run on Bun 1.4 or later: `nomi run` and `nomi test` start `bun`, a built entry is an ES module for Bun, and the runtime's probe refuses an older Bun or another host with a message. Node is no longer a target. Plain Elm programs keep building for the browser exactly as elm builds them. Supporting Node beside Bun would mean two kernels for every module, each tested on both, with Node reimplementing what Bun gives (a WebSocket server, SQL, Redis, S3, argon2).

**D2. Bytes are the unit of I/O.** Every reader, writer, socket, file, body, pipe, hash and blob is `Bytes`, elm/bytes' type, so `Bytes.Encode` and `Bytes.Decode` work on everything. Text is a layer at the edges: `Fs.read`, `Stdio.printLine` and `Request.text` stay, and decode or encode UTF-8 strictly, failing with an error rather than replacing bad bytes. Paths, headers, arguments, environment variables and SQL text stay `String`. `Nomi.Bytes` adds what elm/bytes lacks, as pure functions on kernel code.

**D3. Streams are pull handles.** One pair of opaque types in nomi/core that every source and sink gives:

```elm
read : Reader -> Task Error (Maybe Bytes)     -- the next chunk, Nothing at the end
write : Bytes -> Writer -> Task Error ()      -- succeeds once the sink accepted it
close : Writer -> Task Error ()
```

A write that waits for the sink is the backpressure. An interrupted `read` loses nothing. Transforms are `Reader -> Reader` (`Compress.gunzip`, `Stream.limit`). Text lines are a `LineReader` (`Stream.lines : Reader -> LineReader`, `Stream.line : LineReader -> Task Error (Maybe String)`), clearer to use than lines as chunks. A pure combinator library (`fold`, `map`, `pipe`, `take`) sits on top with no kernel. On Bun, a body is a `ReadableStream` of the default kind, which waits for a slow reader; the `direct` kind buffers whatever is written, so kernels don't use it.

**D4. What `Env` carries.**

| Kind | What | Why |
|---|---|---|
| Capability, in `Env` | `fs`, `net` (connect and listen, one capability, narrowed with `Net.onlyHosts` and `Net.noListen`), `processes`, `stdin`, `stdout`, `stderr` | authority over the world |
| Derived capability | a database, a Redis client, an S3 bucket | made from `net` or `fs` and a config |
| Data, in `Env` | `args`, `vars` | snapshots; `Sql.configFromEnv env.vars` reads them explicitly |
| Ambient task | clock, sleep, secure randomness, logging | no authority; each one controlled by the test runtime |
| Pure function | hashes, HMAC, constant-time compare, encodings, presigning at a given time, compression of small values | deterministic, so no task |

**D5. Fakes.** Capabilities are opaque kernel values, and each module that makes one makes its fake too, so a fake has exactly the real API: `Fs.memory`, `Net.loopback` (an in-memory network on which a program's own server answers its clients), `Processes.fake`, `S3.fromFs`, `Redis.memory`. SQLite in memory is the real thing; Postgres, MySQL, Redis and S3 also run for real in CI, in containers.

**D6. Resources are scoped.** Handles are opened with a function that closes them when its task ends, however it ends, on the runtime's `acquireRelease`; an explicit `close` exists where a program closes early.

**D7. Errors are per module and typed.** Each module has its own error type with `errorToString`, and failures are constructors (`NotFound`, `PermissionDenied`, `ConnectionRefused`, `TimedOut`, a database's SQLSTATE), never strings to match on.

**D8. Packages.** All bundled, versioned together, bumped with nomi:

| Package | Modules |
|---|---|
| `nomi/core` | `Nomi`, `Nomi.Task`, `Nomi.Fiber`, `Nomi.Ref`, `Nomi.Queue`, `Nomi.Bytes`, `Nomi.Stream`, `Nomi.Fs`, `Nomi.Path`, `Nomi.Stdio`, `Nomi.Process`, `Nomi.Crypto`, `Nomi.Random`, `Nomi.Password`, `Nomi.Log`, `Nomi.Compress` |
| `nomi/net` | `Nomi.Net`, `Nomi.Tcp`, `Nomi.Tls`, `Nomi.Udp`, `Nomi.Dns` |
| `nomi/http` | `Nomi.Http.Server` (today's `Nomi.Server`), `Nomi.Http.Client`, `Nomi.Http.WebSocket`, and the pure `Nomi.Http.Route`, `Nomi.Http.Cookie`, `Nomi.Http.Form` |
| `nomi/sql` | `Nomi.Sql` |
| `nomi/redis` | `Nomi.Redis` |
| `nomi/s3` | `Nomi.S3` |
| `nomi/test` | as today, plus inspecting fakes |

`nomi/server` becomes `nomi/http`. The M11 APIs that change (`Stdio`, the server) take 2.0.0, as elm.json's rules require; nobody else uses them yet. The batteries are marked experimental in their docs until a release has run them in production.

**D9. One SQL driver.** `Nomi.Sql` is one module over `Bun.sql`, which speaks Postgres, MySQL and SQLite; `bun:sqlite` isn't used, so there is one SQLite in nomi, not two. What that gives up (loading extensions, custom SQL functions, serializing a database to bytes) is rarely needed by a server and can come later through `Bun.sql` if it grows them. SQLite opens with `safeIntegers`, so 64-bit integers stay exact.

**D10. Routing, cookies and forms ship, as pure Elm,** small and replaceable, in `nomi/http`. Sessions, CSRF, JWT and templating are left to packages.

**D11. Workers are after 0.7.** Fibers give concurrency on one thread; workers running another nomi module, for CPU-bound work, are refined once 0.7 ships.

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
read : Reader -> Task Error (Maybe Bytes)
readAll : { limit : Int } -> Reader -> Task Error Bytes
write : Bytes -> Writer -> Task Error ()
close : Writer -> Task Error ()
pipe : Reader -> Writer -> Task Error ()
limit : Int -> Reader -> Reader
lines : Reader -> LineReader
line : LineReader -> Task Error (Maybe String)
fromBytes : Bytes -> Reader
```

- `Nomi.Stdio`: `stdin` is a `Reader`, `stdout` and `stderr` are `Writer`s; `line`, `print` and `printLine` stay as text helpers. Adds `isTerminal` and `size`.
- `Nomi.Process`: `exit` stays; adds `cwd`, `pid`, `platform`, and `signals : List Signal -> Task x (Queue Signal)`. SIGINT and SIGTERM keep ending the program.
- The runtime formats a defect itself (its name, message and stack), so a report reads the same whatever the host prints.

## 4. Tier 1

**Files (`Nomi.Fs`, `Nomi.Path`).** Adds `stat`, `rename`, `copy` (recursive), `remove` with `{ recursive }`, `makeDirectory` with parents, `symlink`, `readLink`, `realPath`, `temporaryDirectory`, `withFile` opening a `Reader` or `Writer`, and `watch : String -> Fs -> Task Error (Queue Change)`, its events batched and documented as hints. `Nomi.Path` is pure: `join`, `parent`, `name`, `extension`, `normalize`, `relative`, glob matching. `subdir` resolves symlinks before allowing a path. `Fs.memory` fakes it all.

**Processes (`Nomi.Process.Command`).** On `Bun.spawn`, its argument list never a shell string:

```elm
command : String -> List String -> Command
withCwd : String -> Command -> Command
withEnv : Dict String String -> Command -> Command
withStdin : Bytes -> Command -> Command
output : Command -> Processes -> Task Error { status : Int, stdout : Bytes, stderr : Bytes }
spawn : Command -> Processes -> Task Error Child   -- stdin : Writer, stdout/stderr : Reader
wait : Child -> Task Error Int
kill : Signal -> Child -> Task Error ()
```

A child belongs to the fiber that spawned it: interrupting the fiber sends SIGTERM, then SIGKILL after a grace.

**HTTP server (`Nomi.Http.Server`).** On `Bun.serve`. The handler stays `Request -> Task Never Response`, its fiber interrupted when the client goes away (Bun aborts the request's signal). `body : Request -> Reader`, with `bytes`, `text`, `json` and `form` helpers that take a limit. A response is text, JSON, bytes, a `Reader` (SSE with the right content type) or a file. `upgrade : (WebSocket -> Task Never ()) -> Response` makes the connection a fiber that receives and sends messages, `send` waiting while Bun reports backpressure until `drain`. TLS from a certificate and key; graceful shutdown as now. Pure helpers: `Route`, `Cookie` (parse, serialize, sign with HMAC), `Form` (urlencoded, and multipart streaming a `Reader` per part).

**HTTP client (`Nomi.Http.Client`).** Building a request is pure; `send : Request -> Net -> Task Error (Response Reader)` on `fetch`, a status being data, not an error. elm/http keeps working, on `fetch`, for code shared with the browser. `Net.loopback` routes a client's requests to a handler.

**Crypto (`Nomi.Crypto`, `Nomi.Random`, `Nomi.Password`).** Pure: `hash : Algorithm -> Bytes -> Bytes` (SHA-1, SHA-2, SHA-3, BLAKE2), `hmac`, `equalConstantTime`; `hashReader` streaming. Tasks: `Random.bytes`, `Random.uuid`, `Random.uuidV7`, seeded per test under `nomi test`. `Password.hash` and `Password.verify` on `Bun.password` (argon2id, verifying bcrypt too). Keys, signatures and encryption are tier 2.

**Compression (`Nomi.Compress`).** gzip, deflate, brotli and zstd, pure for small values and `Reader -> Reader` for streams.

**Logging (`Nomi.Log`).** `info : String -> List ( String, Value ) -> Task x ()` with levels, and `annotate`, whose fields every line logged in its fibers carries. Text on a terminal, JSON lines otherwise. `Nomi.log` stays.

**SQL (`Nomi.Sql`).** One module, three databases:

```elm
type Value = Null | Int Int | Float Float | Text String | Blob Bytes | Bool Bool
connect : Config -> Net -> Task Error Db               -- Postgres or MySQL, a pool
open : String -> Fs -> Task Error Db                    -- SQLite, a file under the capability's root
memory : Task x Db                                      -- SQLite in memory
query : String -> List Value -> Decoder row -> Db -> Task Error (List row)
execute : String -> List Value -> Db -> Task Error { changes : Int }
transaction : (Db -> Task Error a) -> Db -> Task Error a   -- rolls back on failure or interruption
listen : String -> Db -> Task Error (Queue String)      -- Postgres LISTEN
```

Row decoders read columns by name, as `Json.Decode` reads fields. A query whose fiber is interrupted is cancelled: `Bun.sql` can't cancel one already running, so the kernel runs each on a reserved connection, knows its backend, and cancels it from another (`pg_cancel_backend` on Postgres, `KILL QUERY` on MySQL). Results arrive whole, as `Bun.sql` has no cursors; `query` takes no limit of its own, and a large result is paged in SQL.

## 5. Tier 2 and the batteries

**Blocks.** `Nomi.Tcp` (connect, and listen with a fiber per connection) on `Bun.connect` and `Bun.listen`, `Nomi.Tls` (connect, listen, upgrading a connection), Unix sockets, `Nomi.Udp` on `Bun.udpSocket`, `Nomi.Dns`, a WebSocket client. With these a community package can write a driver for what nomi doesn't ship, in pure Elm.

**Batteries.**
- **Redis** on `Bun.redis`: the common commands typed, `command : List Bytes -> Redis -> Task Error Reply` for the rest, pub/sub as a queue a fiber reads; interruption closes the connection, which cancels a blocking command.
- **S3** on `Bun.s3`: get and put as streams, head, delete, list, presigned URLs; tested against MinIO in a container.

## 6. Order of work

1. **MB.3** Bun as the host (D1).
2. **MB.4** `Nomi.Bytes`.
3. **MB.5** `Nomi.Stream`; `Stdio` on it; nomi/core 2.0.0.
4. **MB.6** Files and `Nomi.Path`.
5. **MB.7** Processes and signals.
6. **MB.8** `nomi/http`: the server on streams, TLS, the client, `Net.loopback`.
7. **MB.9** WebSockets.
8. **MB.10** Routes, cookies and forms.
9. **MB.11** Crypto, random, passwords and compression.
10. **MB.12** Logging.
11. **MB.13** `nomi/sql`.
12. **MB.14** `nomi/net`.
13. **MB.15** `nomi/redis`.
14. **MB.16** `nomi/s3`.
15. **MB.17** Examples on the platform, the README, and an exit review.
16. **MB.18** Release 0.7.
