# Backend platform: draft (B.2)

Status: draft for the maintainer, 2026-10-17. Once agreed, its decisions move into architecture.md §8 and plan MB's steps are written from §6 below. The survey behind it, what Bun, Deno, Go, Gleam, Node, Effect, ZIO and Roc ship and what Node's built-in modules can host, is kept with the development notes (`~/.cache/nomi-dev/mb/survey.md`).

## 1. Principles

1. **The Elm way.** No FFI. Every effect is a function of a `nomi/*` package, typed, documented and tested; only nomi's bundled packages carry kernel code. Everything else, the community's packages included, is pure Elm built on them.
2. **Blocks first, batteries on top.** nomi ships the low-level blocks an ecosystem needs (bytes, streams, processes, files, sockets, TLS, crypto), and then, as Bun does, the batteries a backend reaches for every day (an HTTP server with WebSockets, SQLite, Postgres, Redis, S3). A battery is written on the same blocks a community package would use, so a community driver for a database nomi doesn't ship is possible in pure Elm.
3. **Node's built-ins only.** Whatever nomi ships runs on Node's own modules: no npm, no native addons. What Node lacks (a WebSocket server, Postgres, Redis, S3 signing, XML for S3) nomi writes in its kernels.
4. **Authority is a value.** What can touch the world beyond the program (files, network, processes) is a capability handed to `main` in `Env`, narrowed by ordinary functions, and absent unless passed. A package can only touch what it is handed.
5. **Every capability has a fake.** `nomi/test` runs any program's tasks on fakes, as it already runs `Fs` in memory and time on a virtual clock.
6. **Interruption cleans up.** A fiber interrupted while it holds a resource releases it: a socket closes, a child process is killed, a transaction rolls back, a multipart upload is aborted. Each battery documents this as a guarantee and tests it.

## 2. Decisions

Each decision has a recommendation; those marked **(ask)** are the maintainer's call.

**D1. Node 24 is the floor (ask).** Today's floor is 22.3. Node 22 leaves active support in April 2027, and 24 is the current LTS. 24 gives `node:sqlite` unflagged, `crypto.argon2` (24.7), brotli in `CompressionStream`, `URLPattern` and a stable `fs.glob`; zstd stays behind a probe. The alternative, 22.13 as the floor with probes for the rest, keeps more users but makes password hashing scrypt-only and puts version checks in several modules.

**D2. Bytes are the unit of I/O.** Every reader, writer, socket, file, body, pipe, hash and blob is `Bytes`, elm/bytes' type, so `Bytes.Encode` and `Bytes.Decode` work on everything. Text is a layer at the edges: `Fs.read`, `Stdio.printLine` and `Request.text` stay, and decode or encode UTF-8 strictly, failing with an error rather than replacing bad bytes. Paths, headers, arguments, environment variables and SQL text stay `String`. A new `Nomi.Bytes` adds what elm/bytes lacks, as pure functions on kernel code: `length`, `slice`, `append`, `concat`, `equal`, `indexOf`, `fromString`, `toString : Bytes -> Maybe String`, `toBase64`, `fromBase64`, `toHex`, `fromHex`.

**D3. Streams are pull handles.** One pair of opaque types in nomi/core, which every source and sink returns:

```elm
read : Reader -> Task Error (Maybe Bytes)     -- the next chunk, Nothing at the end
write : Bytes -> Writer -> Task Error ()      -- succeeds once the sink accepted it
close : Writer -> Task Error ()
```

A write that waits for the sink is the backpressure: a fiber writing faster than its socket is suspended. An interrupted `read` loses nothing, as `Stdio.line` already promises. Transforms are `Reader -> Reader` (`Compress.gunzip`, `Reader.limit`), as Go's are. On top, a pure Elm library of combinators (`lines`, `fold`, `map`, `pipe`, `take`), which needs no kernel and could be anyone's; nomi ships the first. The alternative, push streams (events or subscriptions), would bring buffering policies into every API and doesn't fit tasks.

**D4. What `Env` carries.** Capabilities are passed; the rest is ambient or pure.

| Kind | What | Why |
|---|---|---|
| Capability, in `Env` | `fs`, `net` (connect and listen), `processes` (run programs), `stdin`, `stdout`, `stderr` | authority over the world; narrowed by functions such as `Fs.subdir`, `Net.onlyHosts` |
| Derived capability | a database, a Redis client, an S3 bucket | made from `net` or `fs` and a config, so they're capabilities by construction |
| Data, in `Env` | `args`, `vars` | snapshots; `Postgres.configFromEnv env.vars` reads them explicitly, never behind your back |
| Ambient task | clock, sleep, secure randomness, logging, workers running nomi's own modules | no authority; each one is controlled by the test runtime |
| Pure function | hashes, HMAC, constant-time compare, encodings, signature verification, presigning at a given time, compression of small values | deterministic, so no task |

**D5. Fakes.** Capabilities are opaque kernel values, and each module that makes one makes its fake too, so a fake has exactly the real API: `Fs.memory`, `Net.loopback` (an in-memory network on which a program's own server, TCP listener or fake upstream answers its clients), `Processes.fake (Command -> Task x Output)`, `S3.fromFs`, `Redis.memory`. SQLite needs none: `:memory:` is the real thing. Postgres and MySQL run for real in CI.

**D6. Resources are scoped.** Handles are opened with a function that closes them when its task ends, however it ends (`Fs.withFile`, `Sql.transaction`, `Tcp.connect`'s connection living as long as the fiber that holds it), on the runtime's `acquireRelease`. An explicit `close` exists where a program closes early.

**D7. Errors are per module and typed.** Each module has its own error type with `errorToString`, as `Fs.Error` does, and OS failures are constructors (`NotFound`, `PermissionDenied`, `ConnectionRefused`, `TimedOut`), never strings to match on.

**D8. Packages and versions (ask).** Proposed packages, all bundled, versioned together and bumped with nomi:

| Package | Modules |
|---|---|
| `nomi/core` | `Nomi`, `Nomi.Task`, `Nomi.Fiber`, `Nomi.Ref`, `Nomi.Queue`, `Nomi.Bytes`, `Nomi.Stream`, `Nomi.Fs`, `Nomi.Path`, `Nomi.Stdio`, `Nomi.Process`, `Nomi.Crypto`, `Nomi.Random`, `Nomi.Log`, `Nomi.Compress` |
| `nomi/net` | `Nomi.Net`, `Nomi.Tcp`, `Nomi.Tls`, `Nomi.Udp`, `Nomi.Dns` |
| `nomi/http` | `Nomi.Http.Server` (today's `Nomi.Server`), `Nomi.Http.Client`, `Nomi.Http.WebSocket`, and the pure parts: `Nomi.Http.Cookie`, `Nomi.Http.Form`, `Nomi.Http.Route` |
| `nomi/sql` | `Nomi.Sql` (one vocabulary of values, decoders and errors), `Nomi.Sqlite`, `Nomi.Postgres`, later `Nomi.Mysql` |
| `nomi/redis` | `Nomi.Redis` |
| `nomi/s3` | `Nomi.S3` |
| `nomi/test` | as today, plus the fakes' inspection (`Nomi.Test` reads a memory `Fs`, the output of a fake process) |

`nomi/server` becomes `nomi/http`. The M11 APIs this changes (`Stdio` and `Server` move to bytes and streams) take a major version, 2.0.0, which elm.json's rules require; nomi is 0.x and nobody else uses them yet, so now is when it's cheap. The batteries are marked experimental in their docs until a release has run them in production, as Bun marks its own. The alternative, one big `nomi/core`, means a CLI tool pulls in a Postgres driver's documentation and a reason to bump.

**D9. Routing, cookies and forms ship, as pure Elm.** They need no kernel, so they could be the community's, but every server needs them on day one, and they're where the HTTP API is judged. nomi ships small ones in `nomi/http`, pure and replaceable (Deno's `@std/http`, Gleam's wisp). Sessions, CSRF, JWT and templating are left to packages.

**D10. Parallelism is workers running Elm (later).** Fibers give concurrency on one thread. For CPU-bound work, a worker runs another nomi module of the project, and the two talk over a typed channel with encoders and decoders, as Elm's ports do. That is tier 2; SQLite doesn't wait for it (§4).

## 3. Foundation

What every later module is written on, and what changes in M11's APIs.

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

-- Nomi.Stream: the handles (D3), and the first helpers
type Reader
type Writer
read : Reader -> Task Error (Maybe Bytes)
readAll : { limit : Int } -> Reader -> Task Error Bytes
write : Bytes -> Writer -> Task Error ()
close : Writer -> Task Error ()
pipe : Reader -> Writer -> Task Error ()
lines : Reader -> Reader                       -- or a line reader type; see §7
limit : Int -> Reader -> Reader
fromBytes : Bytes -> Reader
```

- `Nomi.Stdio`: `stdin` is a `Reader`, `stdout` and `stderr` are `Writer`s; `line`, `print` and `printLine` stay as text helpers. Adds `isTerminal : Writer -> Bool` and `size : Writer -> Task x (Maybe { columns : Int, rows : Int })`.
- `Nomi.Process`: `exit` stays; adds `cwd`, `pid`, `platform`, and `signals : List Signal -> Task x (Queue Signal)` for a program that reloads on SIGHUP. SIGINT and SIGTERM keep ending the program as now.

## 4. Tier 1: a credible backend

**Files (`Nomi.Fs`, `Nomi.Path`).** Adds `stat` (kind, size, times, permissions), `rename`, `copy` (recursive), `remove` with `{ recursive }`, `makeDirectory` with parents, `symlink`, `readLink`, `realPath`, `temporaryDirectory`, `withFile` opening a `Reader` or `Writer`, and `watch : String -> Fs -> Task Error (Queue Change)`, its events batched and documented as hints. `Nomi.Path` is pure: `join`, `parent`, `name`, `extension`, `normalize`, `relative`, glob matching. `subdir` resolves symlinks before allowing a path, so nothing escapes it. `Fs.memory` fakes it all.

**Processes (`Nomi.Process.Command`).** A builder and two ways to run it:

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

Arguments are a list, never a shell string. A child belongs to the fiber that spawned it: interrupting the fiber sends SIGTERM, then SIGKILL after a grace.

**HTTP server (`Nomi.Http.Server`).** The handler stays `Request -> Task Never Response`. Bodies become streams: `body : Request -> Reader`, with `bytes`, `text`, `json` and `form` helpers that take the limit. A response is text, JSON, bytes, a `Reader` (which serves SSE with the right content type) or a file, and `upgrade : (WebSocket -> Task Never ()) -> Response` turns the request into a WebSocket, its connection a fiber that receives and sends messages; nomi writes RFC 6455 on Node's `upgrade` event, tested with the Autobahn suite. TLS with a certificate and key from bytes; HTTP/2 with TLS, through `node:http2`'s HTTP/1 fallback. Graceful shutdown stays. Pure helpers in the same package: `Route` (paths matched as `List String`, as now, plus a small parser for `"/users/:id"`), `Cookie` (parse, serialize, sign with HMAC), `Form` (urlencoded and multipart, the latter streaming to a `Reader` per part).

**HTTP client (`Nomi.Http.Client`).** Building a request is pure; sending it takes the `Net` capability: `send : Request -> Net -> Task Error (Response Reader)`. A status is data, not an error. elm/http keeps working, on `fetch`, for code shared with the browser. `Net.loopback` routes a client's requests to a handler, so a test calls a fake upstream without sockets.

**Crypto (`Nomi.Crypto`, `Nomi.Random`).** Pure: `hash : Algorithm -> Bytes -> Bytes` (SHA-1, SHA-2, SHA-3, BLAKE2), `hmac`, `equalConstantTime`. Streaming: `hashReader : Algorithm -> Reader -> Task Error Bytes`. Tasks: `Random.bytes : Int -> Task x Bytes`, `Random.uuid`, `Random.uuidV7`, all seeded per test under `nomi test`. Passwords: `Password.hash : String -> Task x String`, a PHC string (argon2id on Node 24, scrypt otherwise), and `Password.verify`. Keys and signatures (Ed25519, ECDSA, RSA), AEAD encryption (AES-GCM, ChaCha20-Poly1305), and HKDF and PBKDF2 are tier 2, with keys as opaque types imported from PEM or JWK.

**Compression (`Nomi.Compress`).** gzip, deflate and brotli: pure for small values, `Reader -> Reader` for streams; zstd behind a probe while Node calls it experimental.

**Logging (`Nomi.Log`).** Structured: `info : String -> List ( String, Value ) -> Task x ()`, with levels, and `annotate : List ( String, Value ) -> Task x a -> Task x a`, whose fields every line logged in its fibers carries (a request's id, for one). Text on a terminal, JSON lines otherwise, chosen by the runtime. `Nomi.log` stays as the simplest line.

**SQLite (`Nomi.Sql`, `Nomi.Sqlite`).** One vocabulary for every SQL driver:

```elm
type Value = Null | Int Int | Float Float | Text String | Blob Bytes
query : String -> List Value -> Decoder row -> Db -> Task Error (List row)
execute : String -> List Value -> Db -> Task Error { changes : Int }
transaction : (Db -> Task Error a) -> Db -> Task Error a    -- rolls back on failure or interruption
```

Row decoders read columns by name, as `Json.Decode` reads fields. `Sqlite.open : String -> Fs -> Task Error Db` opens a file under the capability's root, and `Sqlite.memory` a database in memory. `node:sqlite` is synchronous, so a query blocks every fiber while it runs: the first release runs it on the main thread, as `bun:sqlite` does, which suits the small fast queries SQLite is used for; a database in a worker thread comes with D10 when a measurement asks for it.

## 5. Tier 2 and the batteries

**Blocks (tier 2).** `Nomi.Tcp` (connect, and listen with a fiber per connection, as `Server.listen`), `Nomi.Tls` (connect, listen, and upgrading a connection, which Postgres, SMTP and Redis need), Unix sockets, `Nomi.Udp`, `Nomi.Dns` (lookup, and records by type), the rest of crypto, a WebSocket client, and workers (D10). With these a community package can write a database driver, SMTP or MQTT in pure Elm.

**Batteries, in order:**
1. **Redis**: RESP3 over `Tcp`/`Tls`, pipelining, pub/sub as a queue a fiber reads, reconnecting; the common commands typed, and `command : List Bytes -> Redis -> Task Error Reply` for the rest. No cluster or sentinel at first, as Bun's. About a week.
2. **Postgres**: the protocol's extended queries, SCRAM-SHA-256, TLS, a pool, LISTEN/NOTIFY as a queue, COPY as streams, and cancelling a query when its fiber is interrupted; on `Nomi.Sql`'s vocabulary. Two to four weeks.
3. **S3**: SigV4 signing, get, put, head, delete, list, multipart uploads (aborted on interruption), presigned URLs; tested against AWS, R2 and MinIO. One to two weeks.
4. Later, on demand: MySQL, a key-value store on SQLite in the shape of Deno KV, SMTP.

Each battery's protocol is kernel JavaScript, for speed of parsing bytes; its API, decoders and errors are Elm.

## 6. Order of work

Proposed steps after B.2, each refined in the plan when it starts, and each with tests and its fake:

1. **B.3** Node 24 floor (D1) and `Nomi.Bytes`.
2. **B.4** `Nomi.Stream`; `Stdio` on it; `nomi/core` 2.0.0.
3. **B.5** Files: the rest of `Fs`, `Path`, streaming files, watch, `subdir` hardened.
4. **B.6** Processes: commands, children, signals, terminal size.
5. **B.7** `nomi/http`: the server on streams, TLS and HTTP/2, the client, `Net.loopback`.
6. **B.8** WebSockets, server and client.
7. **B.9** Routes, cookies and forms.
8. **B.10** Crypto, random, passwords; compression.
9. **B.11** Logging.
10. **B.12** `nomi/sql` and SQLite.
11. **B.13** `nomi/net`: TCP, TLS, Unix sockets, UDP, DNS.
12. **B.14** Redis. **B.15** Postgres. **B.16** S3.
13. **B.17** Examples rewritten on the platform (todo-server on SQLite, fullstack with WebSockets), the README's Node section, and an exit review.
14. **B.18** Release 0.7.

Workers, the rest of crypto and the later batteries are refined into steps once these land, or moved past 0.7 if the maintainer prefers a smaller release.

## 7. Open questions

- **Line readers.** `lines : Reader -> Reader` of lines as bytes, or a separate `LineReader` with `line : LineReader -> Task Error (Maybe String)`. The second is clearer to use; the first composes.
- **`Net` split.** One `net` capability for connecting and listening, narrowed with `Net.onlyHosts` and `Net.noListen`, or two in `Env`.
- **Where `Processes` lives.** In `Env` as proposed, or narrowed further to a list of programs a package may run.
- **Workers and SQLite.** Whether a worker-hosted SQLite is in 0.7 or waits.
