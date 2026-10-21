# nomi architecture

Status: proposal, 2026-10-06; redirected 2026-10-20, when nomi became its own language.

nomi is a programming language and its toolchain: compiler, formatter, language server, test runner, package manager and workspaces, and a runtime. It began as a Rust toolchain for Elm. From 2026-10-20 it is its own language, forked from Elm 0.19 at the tag `elm-compat-final`: a backend language compiled ahead of time to a static executable for Linux x86-64. This document covers:

- the direction, and a verdict on the first plan (§1);
- the structural mistakes of the first attempt, with evidence (§2);
- the rules and structure that prevent those mistakes (§3–§13);
- the language (§14) and its native target (§15).

§2 and §4–§13 describe the toolchain as it stood at `elm-compat-final`. Each is rewritten as the F milestones change what it describes; until then, where one of them names Elm's compatibility, upstream parity or the browser, §1, §3, §14 and §15 win.

Anything not settled here should be settled by a test, not by more prose.

## 1. Direction

nomi is a pure, strict, statically typed functional language for servers and command-line programs. Its starting point is Elm: Elm's syntax, its module system, records, custom types and pattern matching, Hindley–Milner inference with extensible records, no exceptions, no null and no mutation of values, and Elm's style of error messages. Its aim is performance: code written the Elm way should run as fast as idiomatic Go, or Rust on tokio, on the same server workloads, and a scoreboard measures it (§15).

**Why a fork.** What makes Elm easy to compile well is its semantics: strict, pure, a closed world, no reflection and no exceptions. What holds it back on a server isn't the language's core but what compatibility pins down:

- its target, JavaScript, where every value is a heap object a garbage collector traces;
- its numbers, where `Int` is a double whose integer arithmetic loses precision past 2^53;
- its strings, UTF-16 with lengths in code units;
- no way to abstract over equality, ordering, hashing or serialization except four special cases, `number`, `comparable`, `appendable` and `compappend`, which no user type joins;
- an effect system of `Cmd`, `Sub` and ports, built for the browser.

A fork can change each of these; a compatible toolchain can change none.

**What changes, from the first plan:**

| Idea | Verdict | Now |
|---|---|---|
| One Rust toolchain | Kept | One parser, resolver and type checker shared by the compiler, formatter, LSP and test runner. |
| CST, HIR and formatter tree | Kept | As §5 and §6 describe. A typed IR follows them (§15). |
| `Task` and fibers on the server | Kept, and the only effect system | `main : Nomi.Program`. `Cmd`, `Sub`, ports, effect modules and `Platform.worker` go. |
| `nomi.toml` selects server or browser | Reversed | One platform: Linux x86-64. No browser. |
| Drop-in replacement for elm, elm-format, elm-language-server and elm-test | Reversed | No compatibility with Elm's compiler, packages or tools. The formatter keeps elm-format's style as nomi's own. |
| Monorepo | Kept | Workspaces as §7 describes. |
| Upstream may ship server modules (Acadia) | No longer a risk | nomi doesn't follow upstream. |

**What doesn't change:** the toolchain's structure (§3–§13), the tolerant phases and the policy layer, the incremental engine, the code rules, and the fiber runtime's guarantees (§8): structured concurrency, interruption, exits, fairness and lifecycle, ported from JavaScript to the native runtime.

**How it gets there.** The JavaScript backend on Bun stays as the program oracle while a reference interpreter is built and checked against it (plan F2); the interpreter then becomes the oracle the native backend is checked against (F4), and the JavaScript backend goes once the native platform covers what it does (F5). Upstream binaries, Compat and every parity suite go first (F1).

## 2. What went wrong in the first attempt

### 2.1 The target changed the language (prevented by R1)

- The backend profile in `standard-library.lock` shipped only elm/core, json, bytes and random. elm/time, url, parser, regex and http were unavailable on the backend.
- The backend `Task` had no `perform` or `attempt`, and the `Platform`, `Platform.Cmd`, `Platform.Sub` and `Process` modules were dropped. That even broke `Platform.worker`, which runs on Node with upstream Elm today.
- A `{Browser, Node}` enum is defined nine times, one of them in the compiler (`MainProfile`, next to `optimize_node`), and `BackendProfile` is defined twice.
- The target is part of the module cache key.

### 2.2 The syntax tree didn't serve its consumers (prevented by R3, §5 and §6)

- Lowering bypasses the typed views: it matches raw kinds (about 120 `NodeKind::` and 70 `TokenKind::` references) and compares token text (`token.text() == ".."`).
- Trivia lands in whichever node is open, so in `x = 1`, `-- about y`, `y = 2` the comment ends up inside `x`'s expression.
- Nodes have no stable IDs.
- The IDE reparses whole files and reimplements scoping (`ide/src/index.rs`, 1,779 lines).

### 2.3 The formatter rediscovered structure (prevented by §5)

- Comments and multiline-ness are found by linear token scans (`tokens_in`, 90 call sites) plus heuristics, all in one 4,313-line file.
- Formatting is quadratic. A 136 KB module takes 2.5 s and a 274 KB module takes 9.9 s, while parsing either takes under 5 ms.

### 2.4 Recovery and robustness were weak (prevented by §5)

- One stray token between declarations turns the rest of the file into a single error node (`unexpected_remainder`).
- Splitting a token such as `=-1` re-lexes the rest of the file, so parse time is quadratic in the number of splits: 43 KB of them take 174 ms to parse.
- There is no recursion guard. 20,000 nested parentheses overflow the parser's stack and 5,000 overflow the formatter's, and either aborts the process.
- A formatter-only grammar mode (36 `ModuleGrammar::Formatting` checks) and a second lexer live inside the syntax crate.

### 2.5 Caching and packages were per-project (prevented by R3, §6 and §7)

- Caches are per project, and the key includes the whole manifest, so any elm.json edit invalidates every module. Packages are compiled separately for each project.
- The full `/all-packages` index is downloaded on every online run.
- The solver is a backtracking search whose only failure message is "package constraints have no solution".
- Kernels are allowlisted by exact (package, version, file SHA-256).

### 2.6 One file could have many owners (prevented by R7)

One file could belong to N projects, plus symlink alias buffers. That policy took about 2k lines of code, 3k+ lines of tests and 11 ADRs.

### 2.7 Runtime correctness bugs (prevented by §8)

- Finished "exposed" fibers are never removed from the fiber table, which is a leak.
- Resource acquire is interruptible.
- Defects get lost.
- Virtual time has bugs.

### 2.8 Ceremony replaced evidence (prevented by R5, §10 and §12)

- Node support is gated on version numbers (24.11+ or 26.5.1+) instead of probing for features.
- 126 tests are `#[ignore]`d until an env var names an exact binary, such as `NOMI_NODE_26_5_1` or a Haskell oracle, so the compatibility evidence never runs by default.
- The paperwork is large: 242 ADRs, PLAN.md at 165 KB, HANDOFF.md at 60 KB and about 29k lines under `docs/`.
- Doc links are hard-coded to 0.19.2.
- Rules written in prose weren't enforced. AGENTS.md gives parallelism to the orchestration layer, yet the formatter crate reads and writes files and builds its own rayon pool (`batch.rs`).

The skeleton of the syntax crate was sound: the event parser, rowan, Missing/Error nodes, flat operator chains, and strict mode implemented as "recover, report the first error". On its 187-file corpus the formatter matched elm-format 0.8.8 byte for byte on 185 files; the other two are reviewed corrections. §11 lists what to keep.

## 3. Rules

These rules are the architecture. Breaking one requires an entry in `DECISIONS.md`. Wherever possible a rule is enforced by the crate dependency graph or a CI check, not by prose. R1 and R5 were rewritten on 2026-10-20, when nomi left Elm's compatibility; the first versions are at `elm-compat-final`.

**R1. One semantics, specified; one platform.** A nomi program means what the language reference (§14) and the reference interpreter say, and nothing else.

- The interpreter is the executable form of the reference. A backend's output is correct when it agrees with the interpreter; where the two disagree, one of them, or the reference, has a bug.
- There is one platform: Linux x86-64, static executables. Another platform is added only by a DECISIONS entry, and it means the same programs the same way.
- Until the JavaScript backend goes (plan F5), it is held to the same semantics, apart from the differences the reference lists, and the compiler has no platform enum.

**R2. Pure core, effects at the edges.** `syntax`, `format`, `compiler`, the IR, the interpreter's evaluator and the backends take values and return values. They touch no filesystem, network, clock, threads or environment variables, and CI checks that they don't depend on crates that do. Only `engine` (through a VFS), `registry` and the runtime perform I/O.

- `unsafe` is allowed in one crate, the native runtime (§15), and in the code the native backend emits. Every `unsafe` block states the invariant it relies on, and the runtime's tests run under Miri where they can and under sanitizers where they can't. Everything else stays `#![forbid(unsafe_code)]`.

**R3. One incremental engine** serves the CLI, watch mode, LSP and tests. It is module-granular and content-addressed, with early cutoff on interface hashes (§6).

**R4. Every phase tolerates errors, and reporting policy is a separate layer.**

- Every phase returns `(output, diagnostics)` and never abandons a module. Unresolved names and failed unifications become error nodes and error types that absorb follow-on errors.
- A policy layer then decides what to show:
  - The CLI shows only errors from the earliest failing phase of each module, and nothing from modules blocked by a failed dependency.
  - The IDE shows everything that's useful.

**R5. Language changes are deliberate, and backends are tested against the reference.**

- A change to the language is a DECISIONS entry, its text in the reference, and the interpreter's tests for it, in that order.
- Every backend runs the same programs as the interpreter, with the same output: the test suites, the examples, the corpus and generated well-typed programs. A difference fails CI.
- A claim about speed is a number on the scoreboard (§15), with its provenance, against the same baseline. A change to the runtime or the backend that makes a scoreboard workload slower beyond the noise needs a DECISIONS entry.

**R6. One source of truth per fact.**

- One manifest per project owns its source directories, dependencies and entries.
- The lockfile, when there is one (plan F7), is generated, never edited.
- One module computes every path.

**R7. One owner per file per view.**

- Each file is canonicalized, with symlinks resolved, to exactly one `FileId`.
- Builds don't need an ownership concept. A module shared by two projects, with identical imported interfaces, gets the same artifact key.
- The IDE gives each file one deterministic owner: the nearest project whose source directories contain it. It never merges diagnostics across owners.

## 4. Crates

| Crate | Owns | Must not |
|---|---|---|
| `base` | `FileId`; interned names; text ranges; `LineIndex` (byte ↔ Elm column ↔ UTF-16); GHC's character classes and case mapping; upstream's suggestion ranking; upstream's reports and their rendering | depend on other nomi crates |
| `syntax` | lexer, parser, rowan CST, generated typed views | know about formatting or semantics |
| `format` | formatter tree, comment attachment, Box renderer | do I/O or spawn threads |
| `compiler` | HIR, canonicalize, types, nitpick, optimize, interfaces | know about platforms, paths or I/O |
| `codegen-js` | JS generation, kernel parser and linker, availability check, source maps | read files (kernels are inputs) |
| `project` | elm's JSON, elm.json, nomi.toml, workspace graph (pure) | touch the network |
| `registry` | registry protocol, solver and its explanations, package store | — |
| `engine` | VFS, build graph, scheduling, caches, cancellation, the only thread pool | contain compiler logic |
| `ide` | problems, hover, completion, navigation, rename and code actions over engine snapshots | use LSP types |
| `lsp` | protocol, position encoding, request queue, debouncing | contain analysis |
| `test` | discovery, runner generation, execution, reporters | — |
| `cli` | `nomi`, plus `elm` and `elm-format` behavior when invoked under those names | contain logic beyond argument mapping |
| `testkit` | parity ratchets, expected results, and the fixtures and harnesses every crate's tests share | depend on nomi crates other than `base`, or be anything but a dev-dependency |

Next to the crates:

- `runtime/` holds the JS kernels and the fiber runtime for each platform.
- `packages/` holds the `nomi/*` Elm packages, which are bundled with the toolchain.

**Diagnostics.** There are two models, one for each shape upstream gives a problem (`DECISIONS.md`, 2026-10-14):

- upstream's reports, base's `Report` and `Problem`: a title, the region as the release keeps it, and a message document (text, code, hints) that phases build rather than format strings. base renders them to the terminal, in upstream's style, and to `--report=json`, in upstream's shape.
- the editor's problems, ide's: a byte range in a file, a severity, the message as elm-language-server writes it, its source and code, and the edits that fix it. lsp maps them to the protocol, and code actions key on their codes.

## 5. Syntax and formatting

### Syntax

**Tree and API**
- The CST is lossless and built on rowan. `parse` always returns a tree plus errors.
- There are three entry points: module, declarations and expressions. The last two are for Elm code blocks in doc comments, which elm-format tries as declarations, then expressions, then a module. The old crate had about 20.

**Tokens**
- Every distinct token gets its own kind (`DotDot`, `ColonColon`, `Minus`, …). Grammar checks never compare text.
- Doc comments `{-| -}` are part of their declaration in both the parser and the formatter.

**Trivia**
- Trivia is emitted lazily. A comment directly above a declaration belongs to that declaration.
- A node never ends with trailing trivia; it bubbles up to the parent. The comment about `y` therefore can't land inside `x`.

**Grammar**
- There is one grammar for every caller. elm's own report of a syntax error comes from a port of upstream's parser, which runs only on a module the grammar rejects, builds no tree, and is held to rejecting exactly what each release rejects (DECISIONS 2026-10-08).
- Where elm-format accepts input the compiler rejects, or the input is legacy syntax, a validation pass flags it instead of a grammar mode. What a project type may use (kernel imports, effect modules) is checked after parsing.
- Layout follows upstream's column rules: fresh line, indented, aligned.
- Operator chains stay flat. Precedence is resolved after canonicalization from the imported fixities, as upstream does.

**Contextual tokens and depth**
- Contextual splits such as `=-` happen in place. Re-lexing stops as soon as the token stream resynchronizes, normally by the end of the line.
- Every recursive pass in every crate, not just the parser, has a depth guard or `stacker::maybe_grow`. Long `++` chains and deeply nested literals lower into deep trees.
- The parser's depth guard also bounds the tree's depth. rowan drops trees recursively, which nothing can guard, so the shell runs core work on threads with `nomi_base::STACK_SIZE` bytes of stack, sized for the deepest tree the guard admits. The tree builder skips rowan's node cache, which rehashes subtrees recursively. Past the limit, the parser reports one error and keeps the rest of the file as a single error node, rather than one error per level as it unwinds.

**Recovery and IDs**
- Recovery at module level resynchronizes at the next column-0 token that can start a declaration, and reports one error per recovery.
- Full reparse is fine (136 KB in about 2 ms). Stable IDs come from the HIR, plus syntax pointers (kind + range) for each revision.
- Typed views are generated from a grammar description, so they're complete, and every consumer uses them.

### Formatter

- **Target.** Byte parity with elm-format 0.8.8 on every input it accepts, and the same rejections.
- **Pipeline.** CST → formatter tree mirroring elm-format's AST → normalizations → Box rendering → text.
  - The formatter tree has elm-format's comment slots (C1/C2) and its multiline flags.
  - Normalizations: imports, exposing lists grouped by `@docs`, literals.
  - Box rendering is a port of `ElmStructure`/`Box`.
- **Comments.** One forward cursor attaches each comment exactly once, to the slot elm-format's parser would use. A debug assertion checks that every comment is emitted exactly once. The whole formatter is linear, and CI benchmarks it.
- **Deviations from elm-format** are allowed only for crashes, non-idempotence, meaning changes or unparseable output. Each one is a fixture plus a line in `DECISIONS.md`.
- **Invariants** (`invariants.rs`), checked on the fixtures, the whole corpus, property tests and the `format` fuzz target:
  - formatting is idempotent;
  - the output parses;
  - the HIR is unchanged (until M5, the tokens apart from elm-format's normalizations, and the imported modules);
  - every comment survives.

  Where elm-format 0.8.8 breaks one, nomi breaks it the same way until plan 2.15 decides otherwise; `invariants.tsv` lists those modules, and fuzzing allows only the classes `upstream_breaks` names.
- **LSP formatting** returns a minimal diff, not a whole-document replacement.
- **Doc comments.** elm-format reads Markdown with its Cheapskate fork, and the format crate ports it (`markdown/`), because a CommonMark parser sees other blocks at the edges. Lowering parses each Elm code block, so rendering never parses. Doc comments get their own differential fixtures.

## 6. Semantic core and engine

### Semantic core

- **HIR.** Each module's HIR lives in arenas with IDs (`ExprId`, `PatId`, `DefId`), plus a source map from IDs to CST pointers. Resolutions, types and diagnostics are side tables keyed by ID. Hover is "ID at cursor → type", the thing the old AST couldn't do.
- **Port upstream's observable logic faithfully:**
  - canonicalization errors;
  - the order in which constraints are generated and solved (error messages depend on it);
  - unification;
  - error construction (`Reporting/Error/Syntax.hs` alone is 5.9k lines);
  - exhaustiveness, decision trees and codegen.
- **Per-expression types.** Recording each expression's type is added without changing that order.
- **Interfaces.** An interface holds the exported values, unions, aliases and binops, and it's hashed. Interfaces are the unit of incrementality.
- **Kernel trust.** Downloaded packages follow upstream's rule unchanged: only `elm/*` and `elm-explorations/*` may contain kernel code. `nomi/*` packages are trusted because they ship inside the toolchain. There are no hash allowlists.

### Engine

**Inputs and artifacts**
- **Inputs:** a VFS (disk plus editor overlays, versioned), manifests, `Compat` and the toolchain version.
- **Unit:** a module in a project context.
- **Key:** a hash of the source, toolchain, `Compat`, the interface hashes of resolved imports and the package identity.
- **Artifacts:** the interface, the optimized graph and the diagnostics.

**Rebuilds**
- **Early cutoff.** If a rebuilt module's interface hash is unchanged, its dependents are skipped. Upstream's `Build.hs` already does this.
- **Scheduling.** Parallelism follows the module DAG on the engine's thread pool, the only pool in the process. It is rayon's, started once, with `nomi_base::STACK_SIZE` for each thread; `cargo xtask ci` fails if another crate depends on a pool crate.
- **Snapshots.** Every edit creates a revision. Readers hold immutable snapshots, and stale work is cancelled between phases.

**Storage**
- **Cache.** One content-addressed, per-user cache that trims itself automatically, like Go's `GOCACHE`. Packages build once per (package, version, resolved dependencies, toolchain), not once per project.
- **Upstream's binary files.** nomi never writes them: `elm-stuff/0.19.x`, `artifacts.dat` and `registry.dat`. That way elm and nomi can share a checkout.

**Why not salsa.** Elm compiles per module, and upstream shows that module-level early cutoff is enough. A module-granular engine is small, persists to disk naturally, and runs the same code for the CLI and the IDE. Revisit only if profiling shows that checking one module per keystroke is too slow.

## 7. Packages and workspaces

**Registry**
- **Index updates.** Use upstream's incremental `/all-packages/since/N` protocol.
- **Downloads.** Run them in parallel and verify each against the hash in `endpoint.json`.
- **Extraction.** Harden it: no absolute paths, no `..`, and size limits.

**Solver**
- **Algorithm.** A port of upstream's search selects, so `install` picks what upstream picks. When it finds nothing, the command reports that as upstream does, without naming a conflict: `install`'s CANNOT FIND COMPATIBLE VERSION, a build's INCOMPATIBLE DEPENDENCIES, `init`'s NO SOLUTION, and reports in their style for nomi's own `update` and the tests' dependencies (DECISIONS, 2026-10-08 and 2026-10-16).
- **Parity.** `install` must pick the same versions as upstream; the Haskell solver oracle tests this.

**Workspaces**
- **Members.** The root `nomi.toml` lists them. Each member is an app or a package with its own elm.json.
- **Resolution.**
  - The whole workspace shares one resolution.
  - A dependency whose name matches a member package resolves to that member's source, with no path configuration.
- **Compatibility with plain `elm`.** Only members that depend on unpublished members need nomi to build. Everything else still builds with `elm`.

**Legacy repos.** Several elm.json files with overlapping `source-directories` work as they are (R7), and nomi suggests turning the shared directories into member packages.

```toml
# nomi.toml at the workspace root
[workspace]
members = ["apps/web", "apps/api", "packages/*"]

# apps/api/nomi.toml
[[entry]]
main = "src/Main.elm"
output = "dist/api.mjs"
platform = "node"   # optional: `main : Nomi.Program` implies node
```

## 8. Platforms, codegen and the server runtime

### Codegen

- **Browser output** is a port of `Generate.JavaScript`: an IIFE with `_Platform_export`, `--debug` metadata and `--optimize` field shortening. Behavior is the contract, and byte-identical output is how it's tested. A deliberate deviation needs a `DECISIONS.md` entry and a behavioral test.
- **Server output** is an ES module for Bun plus a small generated entry, with optional source maps for stack traces.
- **Kernels** keep upstream's chunk format, so one linker serves every platform.

### A server `main` needs zero compiler changes

Upstream accepts any `main` whose de-aliased type is `Platform.Program flags model msg` with a valid flags type (`Optimize/Module.hs`, `addDefHelp`); `()` passes `checkPayload`. So:

```elm
-- nomi/core (bundled with the toolchain, kernel-backed)
type alias Program =
    Platform.Program () () Never

program : (Env -> Task String ()) -> Program
program =
    Elm.Kernel.Nomi.program
```

```elm
module Main exposing (main)

import Nomi
import Nomi.Http.Server as Server
import Task exposing (Task)


main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            Server.listen Server.defaults handle env.net
                |> Task.mapError Server.errorToString


handle : Server.Request -> Task Never Server.Response
handle _ =
    Task.succeed (Server.text 200 "hello")
```

- The type checker and main validation stay untouched, and the kernel starts the root fiber.
- `Platform.worker` programs with ports also run on Bun unchanged, which gives existing code a migration path.
- An entry's platform defaults from `main`'s annotation and can be overridden in nomi.toml.

### Runtime

Elm's `Scheduler.js` already runs processes, bindings with kill functions and a work queue. The server platform ships an extended `Scheduler` kernel with the same Elm-facing API. It adds what Effect's runtime has and Elm's lacks:

| Need | Rule (each one gets a test) |
|---|---|
| Structured concurrency | Every fiber has a parent scope. Closing a scope interrupts its children, then runs finalizers in LIFO order. `Process.spawn` keeps upstream semantics: it is attached to the program root and outlives its spawner. |
| Interruption | Delivered at async boundaries and yield points, and masked in uninterruptible regions. Acquire is uninterruptible, and release runs exactly once. |
| Exits | Success, typed failure, defect or interruption. A JS exception in a kernel call becomes a defect of that fiber. It is reported, never lost, and never crashes the process. |
| Fairness | Yield to the event loop every N steps. |
| Bookkeeping | Finished fibers leave every table, and metrics are read before teardown. |
| Lifecycle | SIGINT or SIGTERM interrupts the root fiber, finalizers run, and the process exits with 130 or 143. HTTP servers drain first: they stop accepting, let in-flight requests finish until a deadline, and only then interrupt what remains. |
| Test clock | Virtual time advances only when every live fiber waits on a timer, never while I/O is pending. |

Two things are deliberately not taken from Effect: the service/layer system and FiberRefs. Elm passes dependencies explicitly, and capabilities are values.

Build this runtime in plain JS first, with model-based tests, in parallel with the compiler. It is the riskiest semantic piece and needs no compiler to test.

### `nomi/*` APIs

- **Concurrency.**
  - Functions: `Fiber.fork`, `join`, `interrupt` and `await`; `Task.both`, `race`, `timeout`, `acquireRelease` and `uninterruptible`.
  - State: `Ref` and `Queue`.
  - `Task.map2` and `Task.sequence` stay sequential as upstream defines them, so parallelism is always explicit.
- **Capabilities.** `Env` carries opaque capability values: `fs`, `net`, `processes` and the standard streams, with `args` and `vars` as data. They can be narrowed, as in `Fs.subdir "data" env.fs`, and each module that makes one makes its fake. Clock, sleep, secure randomness and logging, through a `Logger` value passed explicitly, stay ambient, under the test runtime's control; deterministic work (hashes, encodings) is pure functions. The existing elm/* effects (`Http`, `Time.now`, `Process.sleep`) stay ambient for compatibility.
- **Bytes and streams.** I/O is elm/bytes' `Bytes`, with text helpers at the edges that decode UTF-8 strictly. Every source and sink is a pull handle, `Reader` or `Writer`, a write waiting until the sink accepts it, which is the backpressure. A handle holds state, so whatever makes one, a transform included, is a task, as `Ref.make` is.
- **Resources.** A handle belongs to the fiber that opened it and is released when that fiber ends, however it ends: interruption closes sockets, kills children, cancels Postgres and MySQL queries and rolls back transactions. A SQLite statement runs on the event loop to its end, the interruption taking effect after it. Event sources (file watches, signals, LISTEN, subscriptions) are handles read with a task, bounded, and able to end and fail.
- **HTTP.** A handler is fetch-shaped, `Request -> Task Never Response`. Each request runs on its own child fiber and is cancelled if the client disconnects. Bodies are streams, and a response may upgrade to a WebSocket served by a fiber.
- **No FFI.** As in Elm, every effect comes from the platform's own packages, with typed APIs, and nothing else calls JavaScript; what a backend needs is nomi's to provide (plan MB).
- **Bun is the host.** Server programs run on Bun 1.4 or later, and the kernels are written on Bun's built-ins (`Bun.serve`, `Bun.sql`, `Bun.redis`, `Bun.s3`, `Bun.spawn`, `Bun.password`), never npm; where one falls short of a guarantee above, the kernel makes up for it where it can, and the module documents where it can't. Node is no longer a target (DECISIONS 2026-10-17); a probe at startup refuses another host or an older Bun, and nomi.toml names the platform `bun`.
- **Packages.** `nomi/core` (programs, concurrency, bytes, streams, files, paths, stdio, processes, the network capability, crypto, passwords, logging, compression), `nomi/net` (TCP, TLS, UDP, DNS), `nomi/http` (server, client, WebSockets, and pure routes, cookies and forms), `nomi/sql` (one module over `Bun.sql` for Postgres, MySQL and SQLite), `nomi/redis`, `nomi/s3` and `nomi/test`, versioned together. [drafts/mb-apis.md](drafts/mb-apis.md) holds the APIs as agreed.

**Ergonomics risk.** Chains of `andThen` are verbose. Roc dropped `Task` for effectful functions with `!`, partly for this reason, and nomi can't add syntax without forking Elm. Mitigations:
- helpers such as `Task.fromResult`, `fromMaybe`, and JSON request and response helpers;
- writing real servers early and judging the API by them.

## 9. Test runner, language server, CLI

### Test runner

- Discovery uses types: every exposed value of type `Test` in the test source directories.
- The runner builds through the engine and runs in Bun workers, a browser project's tests included. Output and flags match elm-test (console, JSON and JUnit; `--seed`, `--fuzz`).
- `nomi/test` adds effectful tests that use fake capabilities and the virtual clock.

### Language server

- **Architecture.**
  - An `lsp-server` loop runs requests on the engine's pool against snapshots.
  - Stale requests are cancelled by revision, and diagnostics are debounced.
  - Positions use UTF-8 when the client negotiates it, UTF-16 otherwise, and are converted only in `lsp`.
- **Parity target: elm-language-server 2.10.0.**
  - Diagnostics: compiler errors, its lint rules (unused imports, values, patterns and so on) and elm-review.
  - Hover with types.
  - Navigation: definition and type definition, references, and rename, which also updates exposing lists.
  - Completion, including auto-import and record fields resolved by type.
  - Its code actions, plus elm-review fixes.
  - Editor structure: document and workspace symbols, folding, selection ranges, linked editing and code lens.
  - Its custom requests: finding tests, exposing and unexposing, moving a function, and virtual file contents.
  - Formatting as a minimal diff.
- **After parity:** semantic tokens, inlay hints and signature help. elm-language-server has none of them.

### CLI

- `nomi`'s own commands follow cargo's verbs, and Elm's names work as aliases for the same command, so both habits work. There is no REPL.

  | Command | Aliases | Does |
  |---|---|---|
  | `nomi new [dir]` | `init` | create a project, in the current directory by default |
  | `nomi build` | `make` | compile entries; `--release` is an alias of `--optimize` |
  | `nomi run` | | build and run entries; `--watch` rebuilds and restarts |
  | `nomi check` | | type-check only, with no code generation |
  | `nomi format` | `fmt` | format files, or check them with `--check` |
  | `nomi test` | | run elm-test-style tests |
  | `nomi lsp` | | start the language server |
  | `nomi add <package>` | `install` | add a dependency to elm.json |
  | `nomi rm <package>` | `remove` | remove a direct dependency, and indirect ones nothing else needs |
  | `nomi update [package]` | | move dependencies to the newest versions the constraints allow |
  | `nomi outdated` | | list dependencies with newer versions |
  | `nomi tree` | | print the dependency graph |
  | `nomi bump`, `diff`, `publish` | | package versioning and publishing, as in elm |

- `nomi`'s arguments are parsed with clap. The `elm` and `elm-format` personalities below are parsed by hand, because their help, errors and exit codes must match upstream exactly.
- Installed as `elm` or `elm-format` (via a symlink), it exposes upstream's commands, flags and output formats, including `elm make --report=json`. That serves elm-review, elm-watch, bundler plugins and editors that shell out. The exceptions are `elm repl`, `elm reactor` and elm-format's `--elm-version 0.18`, which exit with a one-line explanation.
- `nomi run --watch` takes the reactor's place: in a workspace it runs every entry in parallel, Bun entries as processes and browser entries behind a dev server, and rebuilds, restarts or reloads only what an edit affects.
- It does not reproduce the order in which upstream's flag parser reports errors.

## 10. Compatibility testing

**Corpora**
- Every package on package.elm-lang.org at its latest version, both compiled and formatted.
- The first attempt's four apps.
- The syntax mutation suite: 39,314 cases from eight seeds.
- An error corpus.
- The first attempt's 605-case `elm make --report=json` matrix.

| Surface | Oracle | Pass criterion |
|---|---|---|
| Accept/reject | Each supported elm release | Identical |
| Diagnostics | `--report=json` | Titles and regions identical under `Compat`. Message text is tracked by a ratchet and must be identical before nomi replaces `elm`, apart from listed deviations. |
| Runtime behavior | Compiled test programs | Identical output |
| Generated JS | `elm make`, `--debug` and `--optimize` | Byte-identical, apart from listed deviations |
| Formatting | elm-format 0.8.8 | Byte-identical, apart from listed deviations |
| Solver | `elm install` | Same versions chosen |

Oracle outputs are stored in the repository as snapshots, or as hashes when they're large, so running the suites never needs upstream binaries. Hermetic suites run on every commit, and the corpus suites need the pinned corpus, fetched once. [plan.md](plan.md) describes the test tiers and the ratchets that track partial parity.

## 11. Salvage from the first attempt

**Test assets (keep):**
- the Haskell parser and diagnostic oracle with its mutation suite (its generator now makes 39,314 cases);
- the elm-format fixtures and package corpus with SHA-256 manifests (185/187 exact);
- the elm-format bug fixtures and the 119 tuple cases;
- the solver oracle and solver tests;
- the LSP transcript harness and record oracle;
- the four app corpora and the 605-case `--report=json` matrix;
- the recovery fuzz target, the `references.toml` pins and the elm-format oracle build recipe.

**Code (keep with light edits):**
- the lexer;
- the event-to-rowan builder, with its losslessness assertion;
- Missing/Error nodes, flat operator chains and `LineIndex`;
- `document.rs` (the Box combinators) and `literal.rs`;
- JS string escaping, `ElmChar(u32)` and BigInt literals;
- the cache entry format and atomic file writes.

**Behavior (reimplement):** the old runtime's lifecycle semantics. That covers:
- exit kinds, including defects;
- cancel acknowledgement and the finishing protocol;
- exit codes 130 and 143;
- graceful drain;
- HTTP hardening.

**References only, not code to copy:** the ported SCC, type solver, kernel chunk parser and JS generator.

**Redesign:**
- **Syntax:** comment ownership, module-level recovery, grammar modes, token splitting and typed views.
- **Formatter:** its overall structure.
- **IDE:** the IDE index.
- **Project and caches:** the project crate and the caches.
- **Runtime:** fiber bookkeeping.

## 12. Build order and process

[plan.md](plan.md) holds the build order: milestones, steps and exit criteria, with one commit per step. nomi replaces one upstream tool at a time, elm-format first, then `elm`, then elm-language-server and elm-test, before adding the server platform and workspaces. Two things start early on purpose:

- A first language server ships with the engine, before type checking. That forces tolerant phases and a single engine from the start.
- The fiber runtime is prototyped in plain JS alongside the compiler, because it needs no compiler.

Process rules:
- **Docs.** This document, plan.md, the README and a `DECISIONS.md` with one paragraph per decision that would be expensive to reverse. If a decision can be a test, write the test.
- **Tests are the spec.** The corpus, the oracles and the snapshots.
- **Fix representations, not symptoms.** If the same class of mismatch needs a third heuristic, the representation is wrong. The old formatter took about 35 rounds of single-mismatch fixes to reach parity, all landing in one file.
- **Small files.** A file past about 1,500 lines gets split by responsibility.

## 13. Code

The tests say what nomi does; this section says how its code reads. Each rule is checked by clippy (the workspace's `[workspace.lints]`) or `xtask ci` where it can be, and by review where it can't. A rule is broken only where upstream parity forces it, and the code says so where it does.

**Types carry the meaning.**
- **T1. No stringly-typed values.** A closed set of values is an enum. Text from outside, such as a JSON field, a command-line word or an event name, is parsed into the enum where it enters and printed from it where it leaves. Strings hold only open text: names, paths, messages and source.
- **T2. No boolean blindness.** A flag whose meaning isn't plain at the call site is an enum with two named variants. So are two or more `bool`s that travel together.
- **T3. Records, not tuples.** A value with three or more parts, or two parts of the same type whose order isn't obvious, is a struct with named fields. A type alias for a tuple is a struct waiting to be written.
- **T4. Illegal states are unrepresentable.** Fields that are only set together are one `Option` of a struct, and fields that exclude each other are one enum. `Option<Option<T>>` and `Result<T, Option<E>>` are named enums.
- **T5. Matches are exhaustive.** A `match` that decides something for each variant of nomi's own enum lists the variants rather than ending in `_`, so a new variant is a compile error wherever it needs a decision. A `_` that means "any other kind", as over the hundreds of `SyntaxKind`s, is fine, and so is a predicate that picks a few variants with `matches!`.
- **T6. Visibility is the smallest that compiles.** An item is private unless another module uses it, and `pub` only when another crate does.

**Functions are small and honest.**
- **F1. Decisions are pure; effects are at the edges.** A function that decides something takes values and returns values, and is tested as such. Loops that read files, wait, print or spawn call it; they don't contain it.
- **F2. One thing per function.** A function past about 80 lines, or nested past four levels, is split along its cases, where a piece names a decision or removes a copy; helpers that only forward, keeping every allocation they had, are no gain. A long `match` whose arms are each short is fine.
- **F3. Signatures say what's used.** A parameter is borrowed unless the function keeps it; a function that can't fail doesn't return `Result`; a parameter, field or `self` that's never read is removed.
- **F4. No needless work.** Nothing is computed only to be discarded, inferred twice, or copied only to be looked at; no `clone` where a borrow does, no `collect` to iterate once, no `format!` pushed into a `String` where `write!` does, no `to_string` of a `&str` that's only borrowed again.
- **F5. Panics state their invariant.** Outside tests, `unwrap` is replaced by `?`, by handling the case, or by `expect` saying why the case can't happen.

**One way to do each thing.**
- **D1. No duplication.** Logic written twice is written once, in the lowest crate that owns the concept (§4), and parallel functions that differ in one detail are one function with that detail as a parameter or a type.
- **D2. No dead code.** Unused items, parameters, fields and variants are deleted, not silenced; `#[allow(dead_code)]` doesn't appear.
- **D3. No speculative generality.** No trait with one implementation, no generic parameter with one instantiation, no option nobody sets, and no wrapper that only forwards, unless a test or a crate boundary needs it.
- **D4. The standard library and established crates first** (rule 9 of the plan), rather than a hand-written equivalent.
- **D5. Tests share their setup.** Cases that share a shape are a table; fixtures and harnesses live in `testkit`.

**Names and comments.**
- **N1. Names say what a thing is**, in the domain's words: upstream's names where nomi ports upstream, Rust's conventions everywhere. No `data`, `info`, `helper`, `util` or `handle` as a whole name.
- **N2. Comments say why, or what upstream does.** Naming the upstream function a piece of code ports (`watcherEventMessage`) is useful. A comment never restates the code, narrates a change or names a plan step: git has the history.
- **N3. Every `pub` module, type and function has a doc comment**, one sentence when one does; a field or variant has one when its name and type don't say enough. No commented-out code.

The code-quality milestone (MQ) changes no behavior: every ratchet, transcript and corpus suite is unchanged by it, and no hot path gets slower. Speed is the business of a profiling pass after the milestones.

## 14. The language

This section is the reference's outline: what nomi keeps from Elm 0.19 and what it changes. Each change is a DECISIONS entry. Where this section says *open*, plan.md names the step that decides it.

**Files and modules.** Source files end in `.nomi`. Modules, imports, exposing lists and layout are Elm's. `infix` declarations stay restricted to the bundled packages.

**Numbers.**
- `Int` is a 64-bit two's complement integer, and its arithmetic wraps. `//`, `modBy` and `remainderBy` by zero give 0, as Elm's do.
- `Float` is an IEEE 754 double. Nothing converts between them implicitly.
- An integer literal has any type of class `Num` and defaults to `Int` when nothing else decides it; a literal with a point or an exponent is a `Float`.

**Text.**
- `String` is immutable UTF-8 text. Its header holds its length in bytes and whether it is all ASCII.
- `Char` is a Unicode scalar value.
- `String.length` and the functions that take indexes (`slice`, `left`, `dropLeft` and so on) count Unicode scalars, as `String.toList` gives them; on an ASCII string they take constant time. `String.byteLength` is the length in bytes, also constant. Grapheme clusters are a module of their own. (Provisional: the maintainer has the last word, plan F0.)

**Equality and classes.**
- `==` requires `Eq`. Functions have no `Eq`, so comparing them is a type error rather than Elm's runtime crash.
- Type classes replace `number`, `comparable`, `appendable` and `compappend`: one type parameter, superclasses, instances by declaration or by derivation. Coherence follows the orphan rule: an instance is declared in the module of its class or of its type.
- Records and tuples have structural instances of the built-in classes their fields have, which the compiler provides and no module can declare. A custom type has an instance only when it derives one or declares one; nothing is derived implicitly.
- Built-in derivations: `Eq`, `Ord`, `Hash`, `Debug`, `Encode` and `Decode`.
- Operators are bound to classes: `+` to `Num`, `<` to `Ord`, `++` to `Append`.
- Open: the syntax of classes and instances (plan F3.1); whether `Ord Float` is IEEE's or a total order (F3.1).

**Polymorphism.** Every program is monomorphized whole (§15), so polymorphic recursion, which Elm allows under an annotation, is rejected with a report naming the cycle.

**Effects.**
- A program is `main : Nomi.Program`. Effects are `Task`s run by fibers, with structured concurrency, interruption, scoped resources and capabilities passed in `Env`, as §8 describes.
- `Cmd`, `Sub`, ports, effect modules, `Platform.worker` and GLSL shaders are gone.
- `Debug.log` writes to standard error, and `Debug.todo` is a defect of the fiber that reaches it.

**Data.** `List`, `Dict`, `Set` and `Array` keep their APIs, with class constraints where Elm had `comparable`. `Array` is a reference-counted vector that an update changes in place when nothing else holds it (§15). `Ref` and `Queue` are the only mutable cells, and only tasks touch them.

**Packages.** The standard library is vendored into this repository from elm/core and the other elm/* packages nomi keeps, under nomi's package names, with their BSD notices in `THIRD-PARTY-NOTICES.md`. Kernel code is Rust in the native runtime and exists only in bundled packages. nomi has its own registry (plan F7); it reads nothing from package.elm-lang.org.

## 15. The native target

**Pipeline.**

```
CST → HIR → canonical → typed (a type for every node)
    → typed IR → monomorphized IR
    → RC IR (explicit dup, drop and reuse)
    → nomi's SSA → machine code → object → static ELF
```

- The **typed IR** is small and explicit: variables, literals, applications of known and unknown functions, constructors, `case` as decision trees, `let`, and class dictionaries as ordinary values. It has its own type checker, run in tests after every pass.
- **Monomorphization** is whole-program: each function exists once per set of types it is used at, and dictionaries become direct calls.
- The **reference interpreter** runs the typed IR. It also runs the RC IR in a checked mode, which counts every reference and fails on a leak, a double drop or a use after free, so the memory model is tested before any machine code exists.
- **SSA** is nomi's own. Machine code comes from Cranelift first, behind that boundary, so that the memory model and the runtime can be measured early; a hand-written x86-64 encoder replaces it only if the scoreboard shows code generation is what's slow.
- **Linking** is nomi's: a static linker for the relocations rustc and the backend emit, combining the program with the runtime's archive. The output is a static executable with no dynamic loader and no libc, carrying DWARF line tables so that perf and gdb work.

**Memory.**
- Reference counting in the style of Perceus: precise `dup` and `drop` inserted by the compiler, borrowed parameters inferred so that a function that only reads a value touches no count, and reuse, where a value whose count is one gives its memory to the value built from it. Pure code updates in place whenever nothing else holds what it updates.
- Data layout follows the types: `Int`, `Float`, `Char` and `Bool` are unboxed words; constructors without fields are small integers; a heap object is one header word, holding the count and the constructor's tag, followed by its fields, which monomorphization lets hold unboxed numbers. The full layout is a plan F4 step.
- Immutable values can't form cycles. `Ref` and `Queue` cells can, and are the only roots a cycle collector would scan (plan F6).

**Runtime.** One `no_std` Rust crate, the only one with `unsafe` (R2).
- **Thread per core, nothing shared.** Each core has its own scheduler, run queue, timers, allocator and I/O ring. A fiber and its children stay on the core that started them, so reference counts are never atomic. A server listens on every core, the kernel spreading connections with `SO_REUSEPORT`.
- **Across cores.** A value sent to another core is copied, or frozen, after which its count is atomic and it is never reused in place. `Ref` and `Queue` belong to the core that made them; a task on another core reaches one by message, which tasks hide, as every access to a cell is already a task. Open: the API that puts work on another core (plan F4).
- **Fibers.** A `Task` is data the scheduler runs, as Elm's scheduler and §8's runtime do; pure code runs on the core's own stack, with a guard page that turns an overflow into a defect of the fiber. A suspended fiber costs its continuation and nothing else, which the 100,000-fiber benchmark measures.
- **I/O** goes through io_uring, and through epoll where io_uring is refused, as Docker's default seccomp profile refuses it; the runtime decides at startup, and a check runs every I/O test on both.
- **Kernels** (strings, bytes, numbers, hashing, and later files, processes and sockets) are Rust functions in the runtime with typed signatures. The first runtime has fibers, the clock, standard I/O, signals and the pure kernels; the rest of §8's platform follows in plan F5.
- §8's guarantees hold unchanged, and the JavaScript runtime's model checker (`runtime/test/check.test.js`) is ported to test the scheduler.

**Scoreboard.** `cargo xtask bench` measures, under the measurement contract in plan.md (F2):
- compute and allocation: binary-trees, n-body, fannkuch, spectral-norm, a `Dict`- and `Set`-heavy workload, JSON encoding and decoding, string building;
- the runtime: 100,000 fibers sleeping and waking, ping-pong across a `Queue`, timers;
- servers: TechEmpower-style plaintext and JSON over HTTP/1.1, latency percentiles under fixed load, memory per idle connection.

Each is written the idiomatic way in nomi, Go, Rust (tokio for servers), OCaml and TypeScript on Bun, plus nomi on Bun as it was at `elm-compat-final`, the baseline every step is measured against.
