# nomi — agreed direction

> Superseded in detail by `docs/architecture.md` (§11 Decision Log); read that first.

Summary of the decisions made on 2026-11-06, before the architecture plan
(`docs/architecture.md`) is written. The plan turns this into a contract; changes after
that go through the decision log.

## 1. What nomi is

**Elm 0.19.3 on the server**, compiled natively to Linux x86-64 static executables, with
performance comparable to C/C++/Rust and as few allocations as possible.

- Elm is the language. nomi does no language design: no typeclasses, no higher-kinded
  types, no new syntax.
- nomi is **not a port of the Elm compiler**. It is our own compiler, written in Rust,
  built from textbook algorithms in the style of the `../algorithm-w` proof of concept.
- Self-hosting is not a goal. The compiler stays in Rust.

## 2. Principles

1. **Elm is the language.** Syntax and semantics come from Elm 0.19.3.
2. **The platform always wins.** Where Elm's behavior is an artifact of JavaScript, nomi
   does what is right for a native target. Every such deviation is written down with its
   reason.
3. **Elm packages are a correctness harness, never a goal.** No decision is justified by
   "package X needs this". Consuming packages is a side effect.
4. **Textbook internals.** Every compiler and runtime stage names the algorithm or paper
   it follows.
5. **No FFI for users.** Only core packages have kernel code (Rust, plus vetted libraries
   such as rustls), behind a wall user code cannot cross. Elm's own model, with Rust as
   the host instead of JavaScript.
6. **Pay only for what you use.** Runtime pieces, tables and libraries are linked only
   when reachable.
7. **Everything is measured.** A scoreboard tracks speed, binary size, compile time and
   memory on every milestone.
8. **Process.** The architecture document is a contract; changes go through a dated
   decision log. Milestones are granular, and each one ends with something that runs and
   is measured.

## 3. Runtime semantics (nomi's own; Elm parity covers checking only)

| Topic | Elm on JS | nomi | Status |
|---|---|---|---|
| `Int` | JS double, exact to ±2^53 | 64-bit, wrapping on overflow | decided |
| `Bitwise` | 32-bit | 64-bit | decided |
| `//`, `modBy`, `remainderBy` by zero | `//` returns 0 (JS `(a / b) \| 0`) | crashes the fiber | decided |
| `String` length and indices | UTF-16 code units | UTF-8 storage, counted in Unicode scalars | decided |
| `String.fromFloat` | JS number formatting | shortest round-trip, format defined by nomi's spec | to specify |
| `==` on functions | runtime crash | compile-time error (monomorphization knows the type) | proposed |
| Ports, `Platform.worker`, `Cmd`, `Sub` | Elm's platform layer | not available; `main : Task Never ()` | decided |

## 4. Compiler (Rust)

Pipeline, each stage with its reference:

1. Lexer → **lossless CST** with Elm's layout rule, written down in our own grammar.
2. AST, modules, name resolution.
3. **Binding groups** by Tarjan SCC.
4. **Type inference: Hindley–Milner, Algorithm J** (union-find, Rémy levels), value
   restriction. Elm's constrained variables (`number`, `comparable`, `appendable`,
   `compappend`) are a closed built-in constraint set. Extensible records by row
   unification (Rémy; Leijen, *Extensible records with scoped labels*).
5. **Pattern matching:** Maranget, exhaustiveness/usefulness (2007) and decision trees
   (2008).
6. **Core IR** in ANF; **whole-program monomorphization**, so nothing is boxed.
7. Core optimizer: inlining under a size budget, then closure conversion and
   defunctionalization.
8. **Perceus** RC insertion (Reinking et al. 2021), borrow inference, reset/reuse
   (*Counting Immutable Beans*, Ullrich & de Moura).
9. **SSA LIR**, kept independent of the instruction set.
10. x86-64 instruction selection, **iterated register coalescing** (George & Appel).
11. GNU `as` + `ld` → static ELF. Our own encoder and ELF writer are an optional later
    step.

The backend (stages 8–10) is ported from `../algorithm-w`. The frontend is new.

**Ownership reports:** the compiler can report every place where reuse or an in-place
update failed, and why. Without this, performance cliffs are invisible.

## 5. Value representation

- Object header: reference count, **shared bit** (from day one), tag, size.
- `Int`, `Float`, `Char`, `Bool` unboxed thanks to monomorphization; nullary constructors
  as immediates; records and tuples flat.
- `String`: UTF-8 with an "all ASCII" flag so common operations stay O(1).
- `Bytes`: RC'd buffer plus offset and length, so slices are zero-copy.
- `Array`: persistent, updated in place when uniquely owned.
- **Acyclic heap:** strict and immutable means no reference cycles, so RC is complete
  with no cycle collector. Process handles are non-owning so this stays true.

## 6. Runtime (`no_std` Rust plus generated fast paths)

- **Allocator:** size-class free lists; the fast path and RC increment/decrement are
  emitted inline by the backend, only slow paths call Rust.
- **Task** is nomi's own (in `nomi/core`) and compiles to direct style
  (`Task e a` ≅ world → `Result e a`), inlined. No interpreter, no node per `andThen`.
  References: Peyton Jones & Wadler (1993), *Concurrent Haskell* (1996), GHC's multicore
  runtime (2009) and Mio IO manager (2013).
- **Fibers** are stackful with **growable contiguous stacks**: since nothing in Elm can
  point into a stack, growing is a plain `memcpy`. The prologue stack-limit check (from
  day one) doubles as the preemption point. `Process.spawn` is a fiber.
- **Scheduler:** single core first. Later M:N, one worker per core, Chase–Lev
  work-stealing deques plus a global queue.
- **IO:** io_uring per worker (epoll fallback); kernel-provided buffers become `Bytes`
  without a copy.
- **Structured concurrency** (Smith, 2018): fibers belong to scopes; cancellation travels
  as a hidden error through normal returns, so no unwinder is needed; resources close
  when their last reference drops.
- **Kernel calls** run on the worker's system stack, never on a fiber stack.
- **Shared state:** processes that own their state (updated
  in place because it is uniquely owned) with mailboxes; possibly a simple `Ref`; STM
  deferred.
- **Multi-core RC:** Lean-style shared bit, atomic operations only for shared objects;
  borrow inference and immortal objects against refcount contention.
- **Crashes** fail the fiber and reach its scope.
- **Airtight effect boundary:** every effect goes through `Task`. That makes deterministic
  simulation testing and replay possible later.

## 7. Server platform (the only API design we do)

- Programs are `main : Task Never ()`; incoming connections arrive as a `Stream`.
- New kernel packages in Elm style: stdio, files, timers, TCP, pull **Streams**,
  **Sinks** (folds that produce a result), bounded **Channels**, later an HTTP server and
  TLS via rustls.
- nomi ships its own standard library (`nomi/core`, `nomi/platform`) in Elm syntax; Elm's
  packages are never shipped and serve only as type-checking test inputs.
- No `use`/do-notation sugar at first; decide after writing CommonMark and the first RESP
  server in plain `|> Task.andThen` style.

## 8. Testing

- **Frontend parity:** accept/reject and inferred types identical to `elm 0.19.3` on the
  corpus (751 packages, 38 apps in `~/.cache/nomi-dev/corpus`, oracles in
  `~/.cache/nomi-dev/oracles`).
- **Semantics:** nomi's own (`docs/semantics.md`); no output comparison with Elm.
- **Differential testing** (as in the POC): reference interpreter versus native at every
  optimization level; RC leak checks on every native test run.
- **External oracles:** CommonMark spec examples; `redis-cli` and the Valkey test suite.

## 9. Stress-test projects

1. **CommonMark → HTML renderer.** Pure, string- and ADT-heavy. Oracle: the spec's ~650
   examples. Yardstick: `cmark`.
2. **RESP key-value server.** Oracles: real clients, Valkey's test suite. Yardsticks:
   Valkey, Dragonfly, a Go RESP server. Grows: strings → expiry → collections → sorted
   sets → pub/sub → transactions → persistence → multi-core.

## 10. Scoreboard

Run time against C references, binary size, compile time (the POC's 14.7 s for 20k
generated lines is too slow), peak memory and allocation counts; later throughput and
p99 latency for the server.

## 11. Milestone sketch (detailed in the plan)

1. Frontend parity on the corpus.
2. Reference interpreter implementing nomi's own semantics (`docs/semantics.md`).
3. Native backend (ported) with Perceus → differential tests pass.
4. Fast `String`, `Bytes`, `Array`, `Dict` → CommonMark passes the spec.
5. Fibers, single-core scheduler, io_uring, server packages → first RESP server.
6. Processes, channels, timers → RESP grows.
7. Multi-core scheduler and shared-bit RC.
8. TLS (rustls), HTTP.

## 12. Known risks

- Invisible performance cliffs when uniqueness is lost (mitigation: ownership reports).
- Allocation-heavy Elm idioms (`List`, `Dict`, closures) at first 2–5× slower than C.
- Compile time under whole-program monomorphization.
- Refcount contention on data shared across cores.
- Sizable kernel work: float formatting, the regex engine.
- Linux x86-64 only (LIR stays ISA-independent).
- Restarting again (mitigation: §2.8).

## 13. Open, to settle in the plan

- Float formatting and `Debug.toString` output.
- Regex semantics.
- `use` sugar, after CommonMark.
- Own assembler and linker, later.
