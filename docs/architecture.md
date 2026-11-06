# nomi — architecture

nomi is a language with **Elm 0.19.3's syntax and type system**, compiled to native
Linux x86-64 executables for servers and command-line programs. It does not take Elm's
platform layer (The Elm Architecture, `Platform.worker`, `Cmd`, `Sub`, effect managers,
ports): a nomi program is a `Task` running on fibers. nomi matches Elm only in the
compile and type-checking phase: which programs are accepted and which types are
inferred. Nothing else is owed to Elm: nomi's standard library (`nomi/core`,
`nomi/platform`) adopts Elm library code and APIs where they make good foundations and
departs from them freely, and nomi may add primitives Elm lacks (a milestone that adds
one extends §6.2 and the §6.13 hash table). Elm's packages as such are inputs to the type-checking parity tests. Runtime behavior is nomi's own, defined by
`docs/semantics.md` (§1.1). The compiler is our own, written in
Rust from textbook algorithms; it is not a port of the Elm compiler. `docs/direction.md`
records how the direction was agreed; this document is the contract. Changes go through
§11 (Decision Log) with a date and a reason.

## 1. Problem & goals

Success means all of the following, each checked by a test or a scoreboard entry:

1. **Language parity.** For every module with an oracle verdict in the parity set (§9.1),
   `nomi check` gives the same verdict as `elm 0.19.3`, and for every exposed top-level
   value the inferred type equals the oracle's after normalization.
2. **Semantic correctness.** Every program of the semantic suite (§9.3) produces the
   output `docs/semantics.md` defines, in the reference interpreter and natively, and
   the two agree on every generated program (§9.5).
3. **Native speed.** The workloads of §10 meet their targets.
4. **Few allocations.** Every workload reports allocation counts (`--stats` builds) and
   peak resident memory; the reuse report lists every allocation site that did not reuse
   memory, with the reason.
5. **Small binaries.** A program links only the runtime parts it reaches; the scoreboard
   tracks the stripped size of every workload executable.
6. **Correct memory.** Every native test that exits normally ends, under `--rc-check`,
   with zero live non-immortal objects (after the exit drain of §6.9) and no access to a
   freed object.

### 1.1 Runtime semantics

nomi's runtime behavior and standard library are its own; Elm's are not a reference. The
semantics are written down in `docs/semantics.md` (first version in M1, completed in M3)
and checked by the reference interpreter and the native matrix. The `nomi/core` APIs are
designed by the milestones that build them. Fixed points:

- `Int`: 64-bit two's complement, wrapping; `minInt // -1` = `minInt`, its remainder 0.
- `Bitwise`: 64-bit; shift counts masked to 6 bits; `shiftRightZfBy` is logical.
- Division, `modBy`, `remainderBy` by zero: crash (§6.12). `Int` `^` with a negative
  exponent: 1 for base 1, ±1 for base −1, crash for base 0, else 0.
- `truncate`, `round`, `floor`, `ceiling`: NaN → 0, saturating at `Int` bounds.
  `String.toInt` beyond 64 bits: `Nothing`.
- `String`: UTF-8; lengths and indices in Unicode scalar values; `compare` in scalar
  order; `Char.fromCode` of a surrogate gives `'\u{FFFD}'`; `Char.toUpper`/`toLower` use
  simple case mapping.
- `String.fromFloat` and Float printing: shortest round-trip (ECMAScript
  Number::toString format).
- `Debug.log` writes `label: value` to stderr; `Debug.toString` prints values in Elm
  source syntax.
- `==`: structural, except for `nomi/core` types that register their own equality
  (§5.6); at a function type a compile error from monomorphization. Polymorphic recursion
  through a binding group is a compile error from monomorphization (`nomi check` accepts
  it).
- Regular expressions: linear-time, ECMAScript syntax without backreferences or
  lookaround (D39).
- Scheduling, cancellation and program end: §6.9–6.10.
- `main : Task Never ()`. Ports are rejected by `nomi build`. `nomi build` resolves only
  `nomi/*` packages and the application's sources; Elm's packages are visible only to
  `nomi check` in the parity harness.

## 2. Non-goals

- Elm's platform layer (The Elm Architecture, `Platform.worker`, `Cmd`, `Sub`, effect
  managers, ports) and Elm's standard library.
- Browser targets, a JavaScript backend, running `elm/browser`, `elm/html`, `elm/svg`,
  `elm/virtual-dom`, `elm/http` or `elm/file` (type-checked for parity only).
- Typeclasses, higher-kinded types, syntax beyond Elm 0.19.3.
- Making third-party Elm packages work. The corpus is a correctness harness.
- Fetching packages, a registry, version solving for user projects.
- Targets other than x86-64 Linux; dynamic linking; position-independent executables.
- Self-hosting. User-level FFI. Catching crashes in user code.
- LSP, formatter, REPL, debugger integration beyond symbols and unwind tables.
- Matching the Elm compiler's error messages or positions.
- Matching Elm's runtime behavior; running programs under Elm and Node for comparison.
- Incremental compilation, except through §10.4. CI on macOS or Windows.

## 3. Constraints

- Rust edition 2024, rustc and cargo 1.99.0; GNU as 2.47 and GNU ld 2.47 (verified).
- Host: x86-64, 24 hardware threads, Linux 7.2.15, `vm.overcommit_memory = 0`,
  `vm.max_map_count = 1048576`, `kernel.io_uring_disabled = 0`, transparent huge pages
  `madvise`, stack ulimit 8 MiB; `gcc`, `perf`, `openssl`, `curl` installed (verified).
  Not installed, needed later: `go` (M6), `redis-cli`, `redis-benchmark`,
  `valkey-server` (M6), `cmark` (M7), `memtier_benchmark` and Dragonfly (M8); each milestone
  plan lists its installs.
- Oracle: `~/.cache/nomi-dev/oracles/elm-0.19.3/elm` (static ELF, prints `0.19.3`), for
  checking only; its `elm repl` starts Node v26.13.0 and needs a writable
  `XDG_CACHE_HOME` (verified).
- io_uring is probed on first use: `io_uring_setup`, then `IORING_REGISTER_PROBE` for
  the required opcodes (read, write, send, recv, multishot accept, timeout, async cancel,
  msg ring, provided buffer rings); anything missing counts as unavailable. If it is
  unavailable, stdio and `File` kernels use
  blocking syscalls on the worker thread, and network kernels print
  `runtime error: io_uring unavailable` and exit 103. With io_uring, the minimum kernel is
  Linux 6.1. Crash reports and exit always use plain `write`/`exit_group`.
- Compiler and algorithm crates: no external crates. Runtime crate: none until M11, which
  adds rustls with `default-features = false`.
- Executables are static and link no libc.

## 4. Current state

### 4.1 This repository

`Cargo.toml` (package `nomi` 0.1.0, edition 2024, no dependencies), `src/main.rs` (cargo
template), `.gitignore` (`/target`), untracked `.agents/skills/` and `.claude/skills/`;
no commits. `docs/direction.md`, `docs/architecture.md`.

### 4.2 The proof of concept `../algorithm-w`

32,611 lines of Rust, no dependencies, 102 commits; a strict ML language with `Int`,
`Bool`, unit, tuples and closures. It has never compiled ADTs, strings or floats (its
D5; `docs/plan-adt.md` is not implemented), and its reset/reuse has only reused tuples.
Pipeline: `syntax/` → `ast/` → `types/` (in-place union-find with Rémy levels, the
imperative Algorithm J, although its plan says W) → `matching/` (Maranget) → `core/` (ANF,
monomorphization, simplifier, occurrence analysis, SAT, worker/wrapper, contification,
interpreter, lint) → `flat/` (closure conversion, CPR, borrow inference, reset/reuse,
explicit RC, RC optimizations, interpreter, lint) → `lir/` (SSA, SCCP, GVN, LICM, DCE,
range analysis, jump threading, verifier) → `x86/` (isel, IRC, naive allocator, frame,
peephole, layout, emission) → `src/x86/runtime.s` (889 lines) → GNU `as` + `ld`. Latest
`docs/bench.md`: 0.30× to 1.07× of `gcc -O2`; 20,000 generated lines to assembly in
7,763 ms.

Facts nomi relies on: header rc u32 / size u16 / `scan_start` / `scan_count`, minimum 3
words (`docs/plan.md:1537-1549`; replaced, §6.1); `extra_args` static array
(`x86/isel.rs:763-779`; replaced, §6.6); slow paths preserve every register except `rax`
(`docs/plan.md:2146-2152`; kept through shims); unwinding for control flow
(`src/guard.rs:46`, `src/core/lower.rs:95,975`, `src/par.rs:95`; kept, D24); CST
`parent` array (`syntax/cst.rs:18`, read only by a test; dropped); mono instance lookup
by linear search with a 1,024-key cap (replaced, §5.6); IRC with `BTreeSet` worklists,
an allocating `adjacent()`, a linear `move_list` scan and a hash set above 8,192 nodes
(`x86/regalloc.rs:32-40,105-151,282-296`; replaced, §5.8); IRC runs for minutes on
functions with hundreds of thousands of values (`docs/plan.md:3115-3118`).

### 4.3 The corpus and the oracle (verified 2026-11-06)

- `~/.cache/nomi-dev/corpus/packages/<author>/<name>/<version>/`: 1,911 packages by 751
  authors, one version each; under each package's own `src/`: 13,719 `.elm` files,
  5,185,827 lines. 226 packages have a dependency whose only corpus version is outside the
  declared range; 15 references name 6 missing packages; `1602/json-schema/4.1.1` has no
  `elm.json`; 183 packages ship a `docs.json`.
- Includes `elm/core 1.0.5`, `json 1.1.4`, `bytes 1.0.8`, `parser 1.1.0`, `time 1.0.0`,
  `random 1.0.0`, `regex 1.0.0`, `url 1.0.0`, the browser packages, and
  `elm-explorations/{benchmark,linear-algebra,markdown,test,webgl}`.
- `apps/`: 38 directories, 121 `elm.json` files outside vendored directories: 117
  applications (101 `0.19.1`, 13 `0.19.0`, 2 `0.19.2`, 1 `0.19.3`) and 4 packages; 17
  directories have only nested projects.
- The oracle rejects an application whose `elm-version` is not `0.19.3`, an application
  without `elm/json`, and any project whose source directories hold modules importing
  `Elm.Kernel.*` (packages with such modules compile only as dependencies). It writes
  `artifacts.dat` and `docs.json` into each package directory it builds.
- `elm repl`, after `import M`, prints `\() -> M.f` as `<function> : () -> M.Model -> M.Model`
  without evaluating `M.f`: aliases by name, names qualified by module except default
  imports (`Int`, `String`, `List`, `Maybe`), constrained variables by class (`number`),
  ANSI colors; about 50 ms per query in one session.
- elm/core references 118 distinct `Elm.Kernel.*` values (`Basics` 38, `String` 31, `List` 7, `Char` 6; `Maybe`, `Result`, `Dict`, `Set`, `Tuple`, `Array` none); `elm/json`, `bytes`, `parser`, `regex`, `time` and `url` together 78.
- A `no_std` staticlib with `_start` and one write syscall, linked with
  `ld -static -nostdlib --gc-sections` and stripped: 4,608 bytes. Cargo rejects `panic`
  in a per-package profile; a `no_std` staticlib fails under an unwinding profile; a std
  crate cannot depend on a crate with `#[panic_handler]`; build scripts run with the
  package directory as working directory; a nested `cargo build` from `build.rs` works and
  reruns for another package only with explicit `rerun-if-changed`.
- Spike S5 (10^6 sleeping fibers, one pooled mapping, 256 bytes touched per stack plus a
  160-byte record): 672 B per fiber with 512 B stacks, 2,208 B with 2 KiB, 4,256 B with
  8 KiB.

## 5. Components

| Crate | Location | Responsibility |
|---|---|---|
| `nomi` | `crates/nomi` (workspace member) | compiler, interpreters, CLI |
| `xtask` | `crates/xtask` (workspace member) | parity, output-parity, fuzz, benchmark, scoreboard |
| `nomi-algo` | `crates/nomi-algo` (`#![no_std]`, no panic handler, workspace member) | pure algorithms shared by the interpreter's and the runtime's kernels (§5.10) |
| `nomi-rt` | `crates/nomi-rt`, its own workspace (in `exclude`), `panic = "abort"` in every profile | runtime staticlib |

`lib/` holds the Elm packages nomi ships; `tests/` test programs and golden files;
`bench/` benchmarks with C and Go references; `apps/` the stress-test projects. Each
compiler stage reads only outputs of earlier stages and never mutates them.

### 5.1 Projects (`project/`)

Reads `elm.json` (`"type": "application"`), resolves `source-directories`, locates
dependencies by the exact versions in `direct` and `indirect`, searching `lib/` and then
`NOMI_PACKAGES` (the parity harness only). `lib/` is embedded in the `nomi` executable
at build time (a generated table of `include_bytes!` files) and read from memory under
the stable path `nomi:lib/...`, so builds do not depend on where `nomi` lives (I10). A
nomi application declares `"elm-version": "0.19.3"` (checked) and lists `nomi/core` and
`nomi/platform` (`"1.0.0"`) in `direct`. Elm's default-import rule applies with the
module list of the package that provides the prelude: Elm's list when `elm/core` is a
dependency (parity harness), `nomi/core`'s list when `nomi/core` is; listing both is a
project error. Modules of the package that declares a prelude (`elm/core`, `nomi/core`)
get no default imports. `nomi/core`'s list is the `prelude` field in its `elm.json`:
`[{"module": "List", "as": null, "exposing": ["(::)"]}, {"module": "Maybe", "as": null,
"exposing": ["Maybe(..)"]}, …]` (`"exposing"` is a list of Elm exposing items or `".."`). No version solving, no network.

### 5.2 Syntax (`syntax/`)

Lexer and recursive-descent parser producing a lossless CST, with operator applications
kept as flat chains that `ast/` re-associates by fixity (fixities are only known after
name resolution); Elm's layout rule as written in `docs/grammar.md` (first version in M1 for the
slice's subset, completed in M2). Parses effect modules, ports and kernel imports. Never
fails: errors become error nodes plus diagnostics.

### 5.3 AST and names (`ast/`)

Per-module arena ASTs; resolves imports, `exposing`, qualified names, operators and
`infix` declarations, default imports; rejects what Elm's canonicalization rejects
(shadowing, duplicate definitions, ports outside port modules, kernel references outside
packages named `elm/*`, `elm-explorations/*`, `nomi/*`; native kernels exist only for
`nomi/*`). Binding groups by Tarjan's SCC.
Interns field names and assigns each a dense `FieldRank(u32)` in UTF-8 byte order.

### 5.4 Types (`types/`)

**Hindley–Milner inference, Algorithm J** (Milner 1978) with in-place union-find and
**Rémy levels**, in one global `TyArena` with per-module scheme tables. Elm's
constrained variables (`number`, `comparable`, `appendable`, `compappend`) as a closed set
of built-in constraints; records with **row unification** (Rémy 1989; Leijen 2005,
without duplicate labels); annotations, rigid variables, recursion, aliases (expanded),
custom and opaque types, `Never`; kernel references have type `∀a. a`, constrained by
their annotations. `Typed` records every reference occurrence's instantiation, keyed by
a global `ExprId(u32) = expr_base[m] + local` and stored as CSR (`inst_start: Vec<u32>`
indexed by `ExprId`, `inst_tys: Vec<TyId>`), defaults already resolved into it. Defaulting
happens once, in `infer`, for variables that stay free at a use site: `number` → `Int`,
`comparable` → `Int`, `appendable` and `compappend` → `String`, other variables → unit.
`lower_core` and the reference interpreter both consume these instantiations.

### 5.5 Patterns (`matching/`)

Maranget: usefulness for exhaustiveness and redundancy (2007), decision trees (2008).

### 5.6 Core (`core/`)

Lowers the typed AST to Core (ANF) with **whole-program monomorphization**:
- Hash-consed concrete types `CTy(u32)`; instance key (definition, interned `CTy` list) in
  an FxHash map; no instance cap and no Core node budget (the POC's caps are dropped).
- Errors: `==`, `compare` or `hash` at a function type; instantiating a definition inside
  its own binding group at a type that is not a renaming of one already on the
  instantiation stack (polymorphic recursion); reachable `Platform.worker`, `Cmd`, `Sub`,
  effect-module values, ports, and kernel references without a table entry.
- Four generated function families per concrete type that needs them, each a
  `Vec<Option<FnId>>` indexed by `CTy`: equality, comparison, `Debug.toString`, and
  **hash** (64-bit, SipHash-1-3 keyed by the process seed of §6.13). A `nomi/*` type
  whose structural form is not its meaning (balanced trees, hash maps, slices) registers
  override functions for three families (equality, `Debug.toString`, hash) in the kernel
  table (§7.3, override rows);
  generated code calls them, monomorphized at the type's arguments. `Float` hashes `-0.0` as `0.0` and every NaN alike. The intrinsic `Elm.Kernel.Hash.hash : a -> Int` lowers to the hash family.
- Lowers `Task` (§6.8) and `main` (§6.9).
- Ported optimizer: occurrence analysis, simplifier with inlining under a size budget,
  SAT, worker/wrapper, contification.

### 5.7 Flat (`flat/`)

Closure conversion with defunctionalization of known calls, constructed-product results
(ported `cpr.rs`), borrow inference, **Perceus** (Reinking et al. 2021), reset/reuse
(Ullrich & de Moura 2019), RC optimizations. Kernel calls follow the kernel table's
ownership modes. Inline frees release only the object itself and hand children whose
count reaches zero to `rt_free`. `rt_free` is iterative with the POC's explicit pending
stack: 64 KiB segments from the span allocator, linked through their first word and kept
for reuse; it continues with the last zero-count child (POC `runtime.s:446-596`). Inline
frees never decrement a closure's `info` field (it is immortal).

### 5.8 LIR and x86 (`lir/`, `x86/`)

SSA LIR and the x86-64 backend (ported): isel; **iterated register coalescing** (George &
Appel 1996) with Appel's flat worklists (a `state: Vec<u8>` plus intrusive doubly-linked
lists in `prev`/`next: Vec<u32>`), an allocation-free `adjacent` iterator, move lists as
sorted vectors, and above 8,192 nodes an open-addressed `u64` edge table
(`lo << 32 | hi`, linear probing, load ≤ 0.5) for membership plus per-node adjacency
`Vec<u32>`; the naive-fallback threshold counts simplify and coalesce work. Frames with
the checks and polls of §6.4, the sequences of §6.5–6.6, GNU assembly with `.cfi_*`
directives. `rbp` is an ordinary register; `r10` and `r11` are reserved for the runtime
sequences and never hold a value across them. TCB loads are volatile in LIR (I15). The
machine-IR verifier runs after register allocation and sequence expansion and checks I5,
I6, I14 and I15. The IRC data-structure changes above are built in M4; M1 ports the
POC's IRC with K = 13.

### 5.9 Driver and CLI (`driver/`, `main.rs`)

Synopsis (`DIR` defaults to the current directory; the program's entry is module `Main`
in the project's source directories):
- `nomi check [DIR]`
- `nomi build [DIR] [-o PATH] [FLAGS]` (default output `DIR/build/<directory name>`;
  `--report reuse` writes the reuse report next to it)
- `nomi run [DIR] [FLAGS] [-- ARGS…]` and `nomi interp [DIR] [--core|--flat]
  [--opt 0|2] [-- ARGS…]`: ARGS become the program's arguments; stdin, stdout and stderr
  are inherited; the exit status is the program's.
- Compiler exit statuses: 0 success, 1 diagnostics reported, 2 usage error, 70 internal
  compiler error.

Build flags: `--opt 0|1|2`, `--regalloc naive|irc`, `--rc-check`, `--stats`,
`--heap-audit`, `--deterministic`, `--force-yield` (every poll and check yields),
`--stack-move` (every `rt_morestack` entry relocates the stack),
`--stack-initial <bytes>`. The driver emits `nomi_config` and `nomi_globals`
(§6.13), writes the embedded runtime archive into a fresh temporary directory, links with
`ld -static -nostdlib --gc-sections`, strips, and renames the result into place.

### 5.10 Interpreters (`interp/`)

- **Reference interpreter**: evaluates the typed AST with its own scheduler implementing
  §6.10, on a thread with a 1 GiB stack (programs needing deeper recursion carry the
  `native_only` flag). Its kernels share their algorithmic cores with `nomi-rt` through
  `nomi-algo` (float formatting, number parsing, UTF-8 scalar operations, JSON
  tokenizing, the regex engine); value marshalling is separate on each side.
- **Core interpreter** (ported): Core at `--opt 0` and `--opt 2`, for optimizer
  bisection. **Flat interpreter** (ported, with its checked heap): Flat at `--opt 2`,
  localizing RC and reuse bugs before the backend.
- The reference and Core interpreters share one `Value` type and the kernel signature
  `fn(&mut Interp, &[Value]) -> KernelResult`, where `KernelResult` is `Done(Value)`,
  `Failed(Value)`, `Pending(WaitToken)` or `Crash(Crash)`; a pending kernel is resumed by
  the interpreter's scheduler with `Done`, `Failed` or cancelled. The Flat interpreter
  keeps its own checked heap cells and calls the same kernels through an adapter that
  converts at the boundary and applies the ownership modes.
- `nomi-algo` uses no `alloc`: every function takes caller-provided scratch slices and
  writes results through an output trait; functions needing scratch proportional to
  their input (the regex engine) first report the size they need.

### 5.11 Shipped packages (`lib/`)

- `lib/nomi/core/1.0.0/`: nomi's standard library in Elm syntax (numbers, strings,
  characters, lists, maybes, results, tuples, collections, bytes, JSON, regular
  expressions, `Task`, `Process`, `Debug`), with kernel references under
  `Elm.Kernel.<Module>`. It declares its prelude (default imports). It starts from
  elm/core's pure Elm modules (`List`, `Maybe`, `Result`, `Dict`, `Set`, `Tuple`,
  `String`, `Char`, `Basics`, whose kernels nomi supplies: about 82 for those modules)
  where they are good
  foundations (BSD-3 licensed, kept in the license file), and changes or replaces any
  of them when the platform calls for it; no compatibility with elm/core is promised.
  Each milestone that adds modules decides their APIs.
- `lib/nomi/platform/1.0.0/`: IO and concurrency (D40). Error types of system kernels are
  built in Elm source from the integer or string codes kernels return.
- `elm/*` packages are never resolved by `nomi build`; code adopted from them lives in
  `nomi/core` as nomi's own.

### 5.11a Module introduction

| Milestone | `nomi/core` | `nomi/platform` |
|---|---|---|
| M1 | `Basics`, `List`, `Maybe`, `Task`, `String` (`fromInt`, `++`) — the slice's subsets; M1 builds both `String` layouts (strings of any length; the index arrives in M4) and the leaf kernels for `fromInt` and `++` through `layout.rs`, starting `tests/layout.rs` | `Stdio.print` |
| M3 | `Basics`, `List`, `Maybe`, `Result`, `Tuple`, `String`, `Char`, `Bitwise`, `Debug`, `Dict`, `Set`, `Random` (pure, seeded generators) (complete) | — |
| M4 | `Array`, `Bytes`, `Json` | `HashMap` |
| M6 | `Task`, `Process` (including a state-owning spawn), `Time` (complete) | `Stdio`, `File`, `Tcp`, `Channel`, `Concurrent`, `Exit`, `Env`, `Signal`, `Stream` (pull core) |
| M7 | `Regex` | — |
| M8 | — | `Stream` (combinators), `Sink` |
| M11 | — | `Tls`, `Http` |

Each milestone designs the APIs of the modules it introduces; the program generator and
the semantic suite cover a module from the milestone that introduces it, in the reference
interpreter and natively alike.

### 5.12 Runtime (`crates/nomi-rt`)

`#![cfg_attr(not(test), no_std)]`; `#[panic_handler]`, `_start` and `no_main` under
`cfg(not(test))`; no libc, no stack protector, no `#[thread_local]`, no `alloc` crate until
M11. System stacks are 8 MiB per worker with a guard page; no runtime or `nomi-algo`
code recurses to a depth set by its input (I18). Provides `_start`, allocator slow paths, `rt_free`, assembly shims (§6.6),
scheduler, fiber stacks, channels, io_uring, timers, signals (§6.14), kernels, `mem*`
functions, crash reporting, `--rc-check`/`--stats`/deterministic support. Rust runs only
on system stacks; generated code runs only on fiber stacks, entered through `rt_enter`.

### 5.13 Harnesses (`crates/xtask`)

`cargo xtask parity`, `output-parity`, `fuzz`, `bench`, `scoreboard` (appends to
`docs/scoreboard.md`). They need the corpus and the oracle; `cargo test` does not.

## 6. Data formats & layout

### 6.1 Object header (compiler ↔ runtime)

| Bits | Field | Meaning |
|---|---|---|
| 0..32 | `rc` (i32) | > 0: count, owned by one fiber; 0: immortal; < 0: shared, count = −rc, atomic (Lean 4 encoding) |
| 32..44 | `size` (u12) | words including the header, exactly as allocated; 0 = large; 4095 = poisoned (`--rc-check`) |
| 44..56 | `scan` (u12) | value fields directly after the header that may hold heap pointers |
| 56..64 | `tag` (u8) | constructor index 0..247; 248 owned `Bytes`; 249 sliced `Bytes`; 250 `String`; 251 raw object (byte buffer, string index, closure info); 252 flat array; 253 runtime record (result slots, in-flight io_uring operation records; M6); 254 foreign-owned raw buffer; 255 extended constructor tag, stored at `p + 8 + 8·scan(p)` (constructor indices ≥ 248) |

- Field order: `scan` value fields first (declaration order; tuples by position; record
  fields by `FieldRank`), then scalar fields. Tuples, records and closures have tag 0.
- `scan(p)` and `size(p)` mean the header fields, or the prefix words when the header's
  `size` is 0; every offset rule in this document uses them.
- Every type-agnostic walk (`rt_free`, immortalization, shared marking, `--rc-check`)
  skips scan words with the low bit set and objects with rc = 0.
- Large objects (more than 4094 words): prefix words `p − 16` size, `p − 8` scan; header
  `size = 0`, `scan = 0`; allocated from a page-granular span allocator. Foreign-owned
  buffers always use the large layout and add a pool word at `p − 24`. Under `--rc-check`
  every object carries one more word before its other prefixes: the allocation-site id
  (u32) and owning fiber id (u32) (`p − 8` for small objects, `p − 24` for large,
  `p − 32` for foreign-owned); header `size` never counts prefix words. Generated code
  stores the site id into `fs:[0x40]` before
  calling `rt_alloc_checked`.
- Minimum object size: 2 words. The empty record `{}` is the scalar 0, like unit.
- Foreign-owned buffers are released only through `rt_free`, which returns them to their
  pool.
- **Allocation, inline**, for n ≤ 64 words:
  `mov rax, fs:[H(n)]; test rax, rax; jz .Lslow; mov r11, [rax + 8];
  mov fs:[H(n)], r11; dec dword fs:[C(n)]; mov r11, <imm64 header>; mov [rax], r11`
  (the free-list link is word 1; generated code writes the header; H and C are in §6.4). `.Lslow` and every n > 64:
  `mov r11d, n; call rt_alloc_slow` (result in `rax`; the caller writes the header of a
  small object; for n > 4094 the caller passes `scan` in `r10` and the shim builds the
  large layout, writing its header with `rc = 1` and the caller's tag passed in the high
  byte of `r10`). Only the refill policy behind `rt_alloc_slow` is delegated.
- **Free, inline** (known layout, `tag ≠ 254`, n ≤ 64): decrement children (handing zero
  counts to `rt_free`; the decrements are explicit LIR operations), then `mov r11, fs:[H(n)]; mov [p + 8], r11; mov fs:[H(n)], p;
  inc dword fs:[C(n)]`. Otherwise `mov r11, p; call rt_free`. When a class's count
  exceeds 4,096 the scheduler moves half of that list to the worker's span allocator at
  its next switch, which returns fully free pages to the OS (M4 decides the page
  bookkeeping, §12).
- **Increment**: rc > 0 → `inc dword [p]`, `jo` to an out-of-line block that saturates
  the object to immortal (`--rc-check` reports it); rc = 0 → nothing; rc < 0 →
  `mov r11, p; call rt_rc_slow_inc`. Emitted as `cmp dword [p], 0; jle 1f;
  inc dword [p]; jo 2f`.
- **Decrement**: rc > 1 → `dec dword [p]`; rc = 1 → free; rc = 0 → nothing; rc < 0 →
  `mov r11, p; call rt_rc_slow_dec`. Emitted as `cmp dword [p], 1; jg dec; je free;
  cmp dword [p], 0; je done; mov r11, p; call rt_rc_slow_dec`.
- Static data (literals with prebuilt string indexes, closure info, constants) lives in
  `.rodata` with `rc = 0`; nothing ever writes to it.

### 6.2 Value representation (after monomorphization)

Constructor index k is the constructor's position in its type's declaration among all
constructors; `Bool` is the one exception (`False` = 0, `True` = 1).

| Type | Word |
|---|---|
| `Int` | i64 |
| `Float` | f64, in SSE registers in generated code |
| `Char` | Unicode scalar value, zero-extended |
| `Bool`, unit, `Order` (`LT` 0, `EQ` 1, `GT` 2), custom types with only nullary constructors | constructor index |
| Single-constructor single-field custom type | the field itself, unless unwrapping reaches the same type again (`type T = T T`, `type A = A B` with `type B = B A`), which is boxed |
| Other custom types | nullary constructor k = immediate `2k + 1`; others point to an object with `tag = k` (`Maybe`: `Just` tag 0, `Nothing` immediate 3; `Result`: `Ok` tag 0, `Err` tag 1) |
| Tuple, record | pointer to an object (§6.1 order) |
| `List a` | `[]` = immediate 1; `x :: xs` = 3-word object with `tag = 1`: `[hdr][head][tail]` when `a` may be a pointer, else `[hdr][tail][head]` |
| `String` | tag 250. Capacity < 256 bytes: `[hdr][byte_len u32 \| scalar_len u32][bytes][slack]`, `scan = 0`; capacity ≥ 256: `[hdr][index][byte_len \| scalar_len][bytes][slack]`, `scan = 1`; data at `p + 16 + 8·scan(p)`; low half of the length word is `byte_len`; capacity from `size(p)` |
| `Bytes` | owned (tag 248): `[hdr][len u64][bytes]`, `scan = 0`; sliced (tag 249): `[hdr][buffer][offset u32 \| len u32]` (low half `offset`), `buffer` a raw or foreign byte object shared by slices (slicing an owned `Bytes` copies it into a raw buffer first); kernels branch on the tag |
| Flat array primitive (`nomi/core`) | tag 252, `[hdr][elements…]`; length = `size(p) − 1`; `scan` = length for element types that may be pointers; the empty array is one static immortal `.rodata` object with `size = 1`, never freed; no spare capacity (a push copies) |
| Function value | tag 0, `[hdr][info][heap captures…][scalar captures…]`; `info` points to a static immortal `[hdr][entry][arity][fast entry]` and counts as a scan field; a partial application is a closure whose captures are the applied closure and the arguments so far |
| `Task x a` | closure (§6.8) |
| `HashMap k v` (`nomi/platform`) | CHAMP (Steindorfer & Vinju 2015), 32-way, single-object nodes: child pointers and the pointer parts of inline key/value pairs first (scan fields), then the scalar parts of the pairs, then one word holding the 32-bit data map and the 32-bit node map; keys' hashes are recomputed when pairs move down, not stored; collision nodes appear only at maximum depth (13 levels of the 64-bit hash) |
| JSON value (`nomi/core`) | a custom type of `nomi/core`; objects keep keys and values in two flat arrays in insertion order, with an open-addressed index of slot indices keyed by the §6.13 seeded hash for objects above 8 keys, computed by one leaf kernel shared by Rust and Elm code |
| Runtime handle | u64 `generation u32 << 32 \| slot u32` into one process-wide slab table per kind (process, timer, file, socket, channel end); a stale generation behaves as closed; a slot whose generation would wrap is retired; a fiber other than the owner using a resource handle (file, socket, timer, channel receive end) gets status 1 with code `not_owner`; channel send ends are capabilities, usable by any fiber, never closed when a fiber ends, and closed explicitly; from M9 lookups read the slot and generation atomically and mutations take a per-kind lock (I13) |

- **String index**: a raw object `[hdr tag=251][u32 entries…]` for non-ASCII strings of
  capacity ≥ 256 bytes, fronting a linear UTF-8 scan: entry i is the byte offset of scalar
  64·(i + 1); there are ⌊(scalar_len − 1)/64⌋ entries (none for an empty string), the
  count is recomputed from `scalar_len`, and a trailing half word is zero. An absent
  index is the immediate 1; in-place append releases the index and writes 1 back.
  Scalar-indexed operations cost up to 64 scalar decodes on non-ASCII strings;
  `nomi/core`'s sequential string APIs (scanning, folding, the parser state) carry byte
  offsets internally. It is written only into
  objects with rc > 0 (on the first scalar-indexed operation); the compiler prebuilds it
  for qualifying literals; the §6.7 walk builds it before immortalization (the index
  becomes immortal too); M9's shared marking builds it before publishing; in-place append
  drops it; `--rc-check` asserts it is never written into an object with rc ≤ 0.
- **String append**: rc = 1 with enough capacity → in place; otherwise a new object with
  twice the needed capacity. Equality, comparison and hashing read only `byte_len` bytes.
- A `String` or sliced `Bytes` longer than 2^32 − 1 bytes crashes with `string too large`.
- RC operations on types mixing immediates and pointers test the low bit first.
- Rejected representations (D45): small-string immediates; pointer tagging in bits 1–2;
  closures with an inline tagged code pointer; flattening product fields into cons cells
  (reopened only if the reuse report shows product allocations dominate a workload).

### 6.3 The heap is acyclic

Elm is strict and immutable; handles are integers; RC is complete, no cycle collector.

### 6.4 Thread control block and fiber stacks (compiler ↔ runtime)

| `fs:` offset | Field |
|---|---|
| `0x00` | TCB self pointer |
| `0x08` | current fiber's stack limit, or `u64::MAX` to force a poll |
| `0x10` | current fiber record |
| `0x18` | system stack top (16-byte aligned) |
| `0x20` | preemption flag |
| `0x28` | shim scratch slot |
| `0x30` | real stack limit of the current fiber |
| `0x38` | allocation counter (`--stats`) |
| `0x40` | allocation-site id for `rt_alloc_checked` (u32) |
| `0x48` | reserved (zero) |
| `0x50..0x90` | runtime-private |
| H(n) = `0x90 + 16·(n − 2)`, n = 2..64 | free-list head for objects of n words |
| C(n) = H(n) + 8 | length of that free list (u32) |

- **Stack geometry**: a fiber stack lives in a power-of-two size class carved from
  per-worker pooled regions (one mapping each, advised `MADV_NOHUGEPAGE`). Its limit is
  `top − usable`, where usable is `--stack-initial` (default 512 bytes) for a new stack
  and the class size minus R after growth; the class is the smallest power of two
  ≥ usable + R, so the default is a 1 KiB class; **R = 512 bytes** lie below the limit,
  its lowest 16 bytes (at `limit − R`, in every class and after every growth) holding
  the canary. The other 496 bytes cover the larger of an
  unchecked frame with its return address and a shim's return address (272 bytes) and
  `rt_morestack`'s register save area with its return address (392 bytes).
- **Checked functions**: every function that calls a non-shim function, has a frame
  above 256 bytes, or contains a loop back-edge or tail-call target starts with `lea r11, [rsp − F]; cmp r11, fs:[8]; jbe .Lmore`, where F
  includes pushes, spills and the largest outgoing and tail-call stack-argument area.
  `.Lmore`: `mov r11d, F; call rt_morestack; jmp` back. Unchecked functions are loop-free,
  call only shims and have frames ≤ 256 bytes (verified), so no poll ever runs below an
  unchecked frame.
- **Polls**: every loop back-edge and tail-call jump target carries
  `cmp rsp, fs:[8]; jbe .Lpoll`, with `.Lpoll: xor r11d, r11d; call rt_morestack` (I14).
- **`rt_morestack`** preserves every register except `r11` and the flags: it pushes all
  15 GPRs, the flags and XMM0–15 (384 bytes) onto the fiber stack, which has R bytes free below the
  limit at every check and poll (only checked functions poll), and stores only `rsp` into
  the fiber record. If the preemption flag is set, it clears it and
  yields. Otherwise it grows only if `rsp − F` is below the real limit (`fs:[0x30]`); it
  then rewrites `fs:[8]` from the real limit and re-checks the flag. The monitor thread
  sets the flag (release) and then stores `u64::MAX` (relaxed), and re-arms on every tick
  while the same fiber keeps running, so a preemption lost to the race is retried.
- **Growth**: a region of the smallest class ≥ used + F + R, `memcpy` of the used part, `rsp` and the saved
  `rsp` moved by the difference. Correct because of I5. `--stack-move` relocates on every
  entry to test this.
- **No guard pages on fiber stacks** (D29). The 16 bytes at `limit − R` hold a canary,
  written in every build and moved with the limit on growth; under `--rc-check` it is verified at each switch, growth, release and
  exit (I16). System
  stacks and the startup stack have guard pages.
- Maximum fiber stack 1 GiB (`stack overflow`).

### 6.5 Stack discipline and Rust calls

- Generated code runs only on fiber stacks; Rust runs only on system stacks.
- **The scheduler is stackless across fiber runs.** It enters a fiber through `rt_enter`
  (assembly: sets `fs:[8]`, `fs:[0x10]`, `fs:[0x30]`, restores the saved register set,
  switches `rsp`). A fiber leaves through `rt_park` (pushes `rbx`, `rbp`, `r12`–`r15`
  onto the fiber stack), a yield in `rt_morestack` (the full set of §6.4) or fiber exit
  (its entry returns into `rt_fiber_exit`, the planted return address); each stores
  `rsp` into the fiber record (about 128 bytes in total: `rsp`, limits, scope, queue and
  timer links) and jumps to a fresh scheduler entry at the system stack
  top, so no system-stack frames are live while a fiber runs.
- **Kernel call sequence**, at every call into a Rust kernel:
  ```
  mov  r11, rsp
  mov  rsp, fs:[0x18]
  push r11
  sub  rsp, 8
  call <kernel>          ; System V AMD64 C ABI; ≤ 6 integer, ≤ 8 SSE arguments
  add  rsp, 8
  pop  rsp
  ```
  For system kernels the sequence then tests `rax` for status 3 (`PENDING`) and calls
  `rt_park`; resumption delivers the operation's `rax:rdx` result. Caller-saved registers
  are dead across the sequence (System V).
- Kernels never block inside Rust, except M1's `Stdio.print` (plain `write` until M6) and
  the blocking stdio and `File` fallback when io_uring is unavailable (§3).

### 6.6 Calling conventions

- **Runtime → generated code** only through `rt_enter` on a fiber stack. `w_main` runs
  `main` (§6.9). Every closure's `info.entry` is `entry(closure, arg) -> rax:rdx` with
  `closure` in `rdi`, `arg` in `rsi` and callee-saved registers preserved as in System V;
  every scalar argument and result, `Float` included, passes as its raw 64-bit pattern in
  general registers. The runtime calls it only on task closures (`arg` = unit 0; status in
  `rax`, payload in `rdx`). `fast entry` and `arity` serve generated code.
- **Generated → generated**: register assignment delegated to M1 (§12), with fixed rules:
  `r10` and `r11` reserved; arguments beyond registers on the stack, never in static
  memory; tail calls remain jumps with a callee-pops convention for stack arguments.
- **Shims** `rt_alloc_slow`, `rt_alloc_checked`, `rt_free`, `rt_free_checked`,
  `rt_rc_slow_inc`, `rt_rc_slow_dec`: assembly in `nomi-rt`; argument in `r11` (and
  `r10` for the second), result in `rax`. `rt_alloc_checked` has `rt_alloc_slow`'s
  contract for every n, small n included (`r11` = n, `r10` = scan for large objects, site
  id in `fs:[0x40]`), places the prefix word itself and returns `p` past it; the caller
  writes the header of a small object as in production. `rt_free_checked` (`r11` = p)
  frees exactly the object: the compiler has already decremented its children inline, as
  in production code. Each saves `rsp` into `fs:[0x28]`, switches to
  the system stack, saves every caller-saved GPR and all SSE registers there, calls Rust,
  restores and returns; it preserves every register except `rax`, `r10`, `r11` and the
  flags, accepts any alignment, and uses 8 bytes of fiber stack.
- **System kernels** return `rax:rdx`: status 0 ok, 1 error (rdx = payload), 2 cancelled,
  3 pending (rdx = 0).
- **Values crossing fibers** (spawned closures, channel messages, results) have the
  kernel-table kind `boxed`: one heap pointer taken `own`. The compiler boxes every value
  whose representation is not always a heap pointer (scalars, constructor indices,
  immediate-or-pointer types) into `[hdr tag=0][value]`, with `scan = 1` when the value
  may be a pointer and `scan = 0` otherwise, and unboxes at the receiving site; the
  runtime releases any `boxed` value through its header alone. From M9 the runtime marks the reachable subgraph:
  it descends from the root through objects with rc = 1, and at the first object with
  rc > 1 on each path marks that object and everything below it shared; it stops at
  objects already shared or immortal; the walk uses an explicit worklist. Before M9 all
  fibers share one worker and nothing is marked. From M6, a sliced `Bytes` whose `len` is below a
  quarter of its buffer, or whose buffer is foreign-owned, becomes an owned `Bytes` when it
  crosses, so retained slices never pin large or ring buffers.

### 6.7 Globals

`_start` sets up the TCB and the hash seed, then enters the main fiber with entry
`w_main` through `rt_enter`. `w_main` is a generated function with no arguments: it calls
the init functions of §6.13 that its build uses, then `w_init`, which evaluates top-level values in dependency order; then calls
`rt_immortalize_globals` through the §6.5 kernel sequence, one iterative walk over the
slots of `nomi_globals` (§6.13) that builds string indexes and makes everything reachable
immortal; then runs `main`, and returns into `rt_fiber_exit`. The main fiber's record is
flagged, so `rt_fiber_exit` on it flushes stdout and exits 0 (after the drain under
`--rc-check`); on any other fiber it ends that fiber.
`--rc-check` counts immortal objects separately.

### 6.8 Task in direct style

`Task x a` is a closure taking the unit word and returning status/value: 0 `Ok a`,
1 `Err x`, 2 cancelled. `succeed`, `fail`, `andThen`, `onError`, `map`, `mapError` lower
to Core code. Calling a task performs its effects. Cancelled propagates like an error no
`onError` observes. M1's acceptance checks that an `andThen` chain allocates no closure
per step after optimization; otherwise the M1 plan adds a Core rule that inlines
`andThen` applied to known closures.

### 6.9 Program entry and exit

- `main : Task Never ()` in the application's main module. `w_main` runs it on the main
  fiber in the root scope.
- When `main` finishes, the program flushes stdout and exits 0; `Exit.withCode n`
  (0 ≤ n ≤ 100; other values crash with `invalid exit code`) records n and cancels every
  fiber, the caller included (it receives status 2 at that kernel and returns through
  ordinary code); the program then exits with n. Every other fiber ends with the
  program. Exits 0 and `Exit.withCode` are normal exits; crash (101), deadlock (102) and
  signal exits are not leak-checked.
- **Under `--rc-check`**, exit first drains: every fiber is cancelled (§6.10), the
  scheduler runs until all fibers have ended and every in-flight io_uring operation has
  completed, then the leak check counts. A fiber still alive after 10^6 scheduler
  switches or 10 seconds prints `rc-check: N fibers alive at exit` and exits 104; tests
  treat that as a failure.
- Elm's platform layer is rejected by `lower_core` when reachable. `Task`, `Process`,
  `Time.now`, `Time.here` and the pure `Random` functions remain.
- Incoming events are tasks and streams from `nomi/platform`: `Tcp.listen` yields a
  `Stream` of connections, `Process.sleep` waits, `Signal` delivers signals as a `Stream`.
- Exit statuses 101 (crash), 102 (deadlock), 103 (io_uring unavailable for networking),
  104 (`--rc-check` drain failure), 105 (leak or heap-audit failure), 141 (stdout closed:
  `EPIPE`, not a signal) and 128 + n (signal) are reserved.

### 6.10 Scheduling, scopes and cancellation

- **Order**: one FIFO run queue. `Process.spawn` enqueues the child at the tail and the
  parent continues. A fiber runs until it parks (channel, IO, `sleep`), yields
  (preemption), or ends; operations that complete immediately (`Time.now`, `Time.here`,
  `kill`) do not yield. Deterministic builds disable preemption. The reference
  interpreter implements the same order.
- **Scopes** (Smith 2018): every fiber belongs to a scope. `Process.spawn` adds the child
  to the root scope. `nomi/platform`'s `Concurrent` combinators (`all`, `race`,
  `timeout`) open a scope for their children; the combinator returns only after all of
  them have ended, and leaving early (race won, timeout, cancellation) cancels the rest
  first. Cancelling a fiber cancels the scopes it has open. The root scope ends with the
  program (§6.9). Since cancellation takes effect at kernel boundaries, `timeout` and
  `race` around pure computation wait for it to reach one; `docs/semantics.md` states
  this and a semantic test shows it.
- **Kill and cancellation**: a fiber that has not started is dropped (its closure is
  released); a fiber queued or parked at a kernel boundary resumes with status 2 instead
  of its result, so no pure code after that kernel runs; a preempted fiber continues its pure code to the next kernel and receives
  status 2 there. An in-flight io_uring operation is cancelled with
  `IORING_OP_ASYNC_CANCEL`; the fiber resumes with status 2 when the original completion
  arrives or after 100 ms, whichever is first. The operation keeps a reference to every
  heap object it touches and a runtime-owned slot for its pointer arguments until its
  completion arrives, so late completions never touch freed memory.
- **Resources** are owned by the fiber that opened them, close through explicit tasks or
  when that fiber ends; `nomi/platform` APIs that hand a resource to a new fiber transfer
  ownership explicitly.
- **Deadlock**: if the main fiber is parked, the run queue is empty, no io_uring
  operation is in flight, no timer is armed and no fiber awaits a `Signal` stream, the
  program prints `runtime error: deadlock: N fibers blocked` and exits 102.

### 6.11 Channels and processes owning state

- `Channel` (in `nomi/platform`): bounded (capacity ≥ 1; a smaller capacity crashes with
  `invalid channel capacity`), multi-producer, single-consumer; a send to a full channel
  parks the sender, and parked senders are admitted in FIFO order; a receive from an
  empty one parks the receiver; when every send end is closed, a parked or later receive
  gets status 1 `closed` once the buffer is empty; when the receive end is closed, parked
  and later sends get status 1 `closed` and their messages are released; ends are
  handles. Kernels: `rt_channel_new(capacity) -> handles`, `rt_channel_send(end, ptr) ->
  status`, `rt_channel_receive(end) -> status:ptr`, all system kernels.
- A process owning state is a fiber looping on a channel receive with its state at
  rc = 1, so updates happen in place. The RESP server keeps its keyspace in N shard
  processes (N = workers), routing keys by hash. From M9, fibers created as state-owning
  processes are pinned to their worker; other fibers may be stolen.

### 6.12 Crashes

`Debug.todo`, division by zero, out of memory, `stack overflow`, `string too large`,
`invalid exit code`, or an internal error: the runtime writes `runtime error: <message>`
to stderr with a raw `write`, flushes stdout (non-blocking, at most 100 ms) unless the
crash interrupted a stdout write, and calls `exit_group(101)`. `nomi/platform` streams
enforce a per-connection input bound, so oversized requests fail as errors.

### 6.13 Build configuration (compiler → runtime)

- `nomi_config` (read-only, emitted by the compiler), 16 bytes: `u32` format version (1)
  at 0; `u8` `rc_check` at 4, `stats` at 5, `deterministic` at 6, `stack_move` at 7;
  `u32` `stack_initial` at 8; `u8` `force_yield` at 12, `heap_audit` at 13; `u16` zero at
  14. `deterministic` builds read the scheduler seed from `NOMI_SCHED_SEED` (default 0);
  other builds never read it.
- `nomi_globals`: `u64` count, then one `u64` slot address per global whose type may hold
  a pointer.
- `nomi_sites` (`--rc-check` builds): `u32` count, then per site `u32` file index, `u32`
  start, `u32` end; then a `u32` count and NUL-terminated file paths relative to the
  project root (I10).
- `nomi_hash_seed`: `[u64; 2]` in `.bss`, defined by the runtime, written by `_start`
  before `w_init`; generated hash code reads it.
- `--rc-check` changes code generation: inline allocation and free become
  `rt_alloc_checked`/`rt_free_checked`, which assert I11, fill freed fields with
  `0xDEAD_DEAD_DEAD_DEAC` (even and non-canonical, so stale field loads fault instead of
  reading as immediates), use the quarantine FIFO and record site and fiber ids (§6.1),
  so leaks report positions. Checked builds also emit a call from `w_main` to
  `rt_rc_check_init` (and `--stats`, `--heap-audit` and `--deterministic` builds to
  `rt_stats_init`, `rt_audit_init`, `rt_det_init`), which registers the drain, the
  reporters and the deterministic scheduler; nothing reachable from `_start` references
  them otherwise, so `--gc-sections` drops them from production executables.
  `nomi_sites` is emitted in every build (count 0 outside checked builds). Generated code
  stores the call site into `fs:[0x40]` before kernel calls in checked builds, so Rust
  allocations carry the caller's site.
- `--stats` increments `fs:[0x38]` on every allocation.
- The process hash seed (128 bits) keys JSON hash indexes, `HashMap` and shard routing.
  If `NOMI_HASH_SEED` (a decimal u64) is set, every build and the interpreter use the two
  words drawn from SplitMix64 of it; otherwise `getrandom` (`GRND_NONBLOCK`; if that
  fails, the time stamp counter mixed with the pid and a stack address).
- **Hash stream.** The hash of a value is SipHash-1-3, keyed by the seed, over a sequence
  of 64-bit words, serialized little-endian:

  | Value | Words |
  |---|---|
  | `Int`, `Char` | the value |
  | `Bool`, `Order`, types with only nullary constructors | the constructor index |
  | unit, `{}` | 0 |
  | `Float` | its bits, with `-0.0` as `0.0` and every NaN as `0x7FF8000000000000` |
  | single-constructor single-field type | the field's words (no index) |
  | other custom types | the constructor index, then the fields' words in declaration order |
  | tuples | the fields' words by position |
  | records | the fields' words in `FieldRank` (field-name) order |
  | lists, flat arrays | the length, then each element's words |
  | `String`, `Bytes` | the byte length, then the bytes packed little-endian into words, the last word zero-padded |
  | override types | the words of the value their override designates as canonical (`Dict`: its sorted list of pairs) |

  `nomi-algo` holds the one implementation of SipHash and these rules for the
  interpreter; the generated hash family follows the same table (I17).
- **Reports.** `--rc-check`, `--stats` and `--heap-audit` results are written at exit to
  the file named by `NOMI_REPORT_PATH`, never to stdout or stderr, one `key value` line
  each (`allocations N`, `live N`, `immortal N`, `leak <file>:<start>-<end> N`,
  `heap_audit ok` or `heap_audit fail <detail>`). A leak or heap-audit failure exits 105;
  without `NOMI_REPORT_PATH`, its one-line summary goes to stderr.
- `--heap-audit` keeps the production inline sequences and adds one counter instruction to
  each: `inc qword fs:[A(n)]` after an inline allocation and `inc qword fs:[A(n) + 8]`
  after an inline free, with A(n) = `0x480 + 16·(n − 2)`; the runtime's own allocation
  paths count into the same slots. After the exit drain it asserts allocations − frees =
  immortalized objects per class, and walks every free list checking that each node lies
  in a span of its class.

### 6.14 Signals

- `SIGPIPE` ignored; socket sends use `MSG_NOSIGNAL`.
- `SIGSEGV`: `SA_ONSTACK`, one alternate signal stack per thread; a fault in a guard page
  of a system or startup stack is `stack overflow`, any other fault an internal error
  with address and `rip`; both exit 101.
- Before M6, `SIGINT`/`SIGTERM` keep their default disposition. From M6 they are blocked
  in every thread and received by the monitor thread with `sigwaitinfo`; the monitor wakes
  an idle worker through an eventfd read each worker keeps armed in its ring (or, without
  io_uring, by acting itself); if a worker has not acknowledged within 1 second, the
  monitor flushes stdout (bounded) and calls `exit_group` itself. The monitor also enforces
  the `--rc-check` drain deadline (§6.9). With a `Signal` stream awaited, signals are
  delivered to it; otherwise the scheduler stops at its next switch, flushes stdout and
  exits 128 + signal number.
- Preemption uses no signals (§6.4). Every exit uses `exit_group`.

### 6.15 Compiler data structures

Ported with the POC's layout rationale (POC `docs/plan.md` §6): token `Vec` with a
non-trivia index; preorder CST struct-of-arrays (`kinds`, `spans`, `subtree_end`);
interned symbols; arena ASTs with `IdRange` lists; in-place union-find types with epoch
marks; per-pass Core arenas with dense renumbering; LIR with `Nop` tombstones and CSR use
lists. Changes and additions:
- `Span { file: FileId(u32), start: u32, end: u32 }`.
- Modules in topological order, `ModuleId(u32)`, module-local ids; global
  `DefId(u32) = module_base[m] + local`; every side table from inference on is a flat
  `Vec` indexed by global `DefId`.
- Record rows: arena lists of `(FieldRank, TyId)` sorted by rank plus an optional tail;
  unification merges two sorted lists comparing integers. No emitted order depends on
  `Symbol` ids (I10).
- Monomorphization tables of §5.6; the zonk memo is epoch-invalidated whenever the
  substitution changes and lives for one lowering.
- IRC structures of §5.8.
- Every table keyed by a dense id (`CTy`, `VarId`, `FVar`, `LamId`, `ExprId`, `DefId`) is
  a `Vec`, including the POC's `FxHashMap`s keyed by them.
- No other caches, except one added under §10.4.

## 7. Interfaces

### 7.1 Compiler stages

```rust
fn parse(file: FileId, src: &str, diags: &mut Diagnostics) -> Cst;
fn load(dir: &Path, sm: &mut SourceMap, diags: &mut Diagnostics) -> Option<Project>;   // parses every module once; owns CSTs and the interner
fn lower_modules(project: &mut Project, diags: &mut Diagnostics) -> Asts;              // per-module ASTs plus global id bases
fn infer(asts: &Asts, project: &Project, diags: &mut Diagnostics) -> Typed;           // instantiations and defaults per occurrence
fn check_matches(asts: &Asts, typed: &Typed, diags: &mut Diagnostics) -> Matches;
fn lower_core(asts: &Asts, typed: &Typed, matches: &Matches, project: &Project, main: DefId, diags: &mut Diagnostics) -> core::Program;
fn optimize_core(p: core::Program, opt: OptLevel) -> core::Program;
fn lower_flat(p: &core::Program, diags: &mut Diagnostics) -> flat::Program;   // reports layout limits (I11)
fn optimize_flat(p: flat::Program, opt: OptLevel) -> flat::Program;
fn lower_lir(p: &flat::Program, cfg: &LirConfig) -> lir::Program;   // rc_check, stats
fn optimize_lir(p: lir::Program, opt: OptLevel) -> lir::Program;
fn emit_x86(p: &lir::Program, cfg: &BackendConfig) -> String;        // GNU assembly
```

Stages after `infer` run only without errors. Every IR has a lint or verifier after each
pass in debug builds and in all tests (I2). `Typed` keeps per-module scheme tables so a
§10.4 cache can serialize one module's interface.

### 7.2 Diagnostics

`Diagnostic { code: u16, severity, primary: (Span, String), secondary: Vec<(Span, String)>, message: String, notes: Vec<String> }`;
codes in `docs/diagnostics.md`, snapshot-tested.

### 7.3 Kernel table

`crates/nomi/src/kernel/table.rs`, keyed by (kernel module, name), for example
(`Elm.Kernel.String`, `length`) or (`Elm.Kernel.Stdio`, `print`), all from `nomi/*` packages. Per entry: class;
for intrinsics a type scheme and a per-instance lowering; for leaf and system kernels a
monomorphic signature and a symbol (`k_<module>_<name>`). A system kernel whose
signature has parameters P and result `Task x a` lowers to a task closure capturing P;
the Rust call happens when the task runs, and `rdx` holds the `x` or `a` value in its §6.2
representation (scalars as raw bits). Ownership per parameter
(`own` consumed / `borrow`, by position); result ownership (`fresh` /
`borrowed-from k`).

| Class | Runs where | Rules |
|---|---|---|
| Intrinsic | inlined | arithmetic, comparison, `Basics`, `Bitwise`, `Hash.hash`, Task and scheduler primitives, `List.cons`, all flat-array operations, `HashMap` node operations |
| Elm source | ordinary code in `nomi/core` or `nomi/platform` | operations that call nomi code or are polymorphic in a varying representation (sorting with a comparator, JSON decoders, `HashMap`) |
| Leaf kernel | Rust, system stack, returns immediately, O(input) | argument types `Int`, `Float`, `Char`, `String`, `Bytes`, raw buffers; may construct only the closed result set below |
| System kernel | Rust, system stack, may return `PENDING` | syscalls, io_uring, timers, channels, scheduler services |

Rust constructs only this closed set of compound values: `Maybe` and `List` of a leaf
type, 2-tuples of leaf types, the JSON value, regular-expression match records, flat
arrays of a leaf type. Their layouts come from `crates/nomi-rt/src/layout.rs`, a dependency-free file
that `crates/nomi` also includes (`#[path]`); the test `crates/nomi/tests/layout.rs`
builds each value through the compiler's layout code and through `layout.rs` and
compares the words. Override rows map (package, module, type name, family) to (module,
function name), resolved to `DefId`s by `lower_core` and monomorphized at the type's
arguments, for three families only (`comparable` admits no `nomi/*` custom type): equality `T -> T -> Bool`, `Debug.toString` `T -> String`, and
hash as a canonical projection `T -> C`, whose value's words the generated hash and
`nomi-algo` splice in place of T's; C never contains T. Kernel parameter and result kinds
are monomorphic types or `boxed` (§6.6), so polymorphic system kernels (channel send and
receive, result slots) take and return `boxed` values. Scheduler services listed in §7.4
are named `rt_*`; every other kernel `k_<module>_<name>`. Leaf kernels never return
`Maybe` of a tuple; `nomi/core` composes such results in Elm source. `nomi check` gives table-less kernel references `∀a. a`;
`nomi build` reports reachable ones. Inference types every kernel reference as `∀a. a`
constrained by its annotation; the table's schemes serve lowering and the I7 test. Native kernels exist only for packages under `lib/`
(I9).

### 7.4 Runtime entry points

Shims: `rt_alloc_slow`, `rt_alloc_checked`, `rt_free`, `rt_free_checked`,
`rt_rc_slow_inc`, `rt_rc_slow_dec`. Fiber control: `rt_morestack`, `rt_park`,
`rt_enter`. System kernels: `rt_spawn(closure, flags) -> handle` (the closure is always a
compiler-generated wrapper of type `Task Never ()` that consumes the user task's result,
writing it, boxed and `own`, into a runtime-owned result slot when a `Concurrent`
combinator waits for it; flag bit 0 marks a state-owning process, pinned from M9),
`rt_kill(handle)`,
`rt_channel_new`, `rt_channel_send`, `rt_channel_receive`, scope operations for
`Concurrent`. Other: `rt_immortalize_globals`, `rt_crash(msg, len) -> !`. Emitted by the
compiler: `w_init`, `w_main`, `nomi_config`, `nomi_globals`, `nomi_sites`. `_start`
enters `w_main` on the main fiber (§6.7); `rt_fiber_exit` is the return address of every
fiber entry.

## 8. Invariants

| # | Invariant | Runtime check | Test |
|---|---|---|---|
| I1 | The CST is lossless | none | every corpus file and fuzz input round-trips |
| I2 | Every IR is well-formed after every pass | lint/verifier after each pass in debug builds | all tests use debug compiler builds |
| I3 | The heap is acyclic, so RC frees everything | `--rc-check`: zero live non-immortal objects after the exit drain (crashes exempt) | every native test under `--rc-check` |
| I4 | No access to a freed object | `--rc-check`: poisoned headers and fields, quarantine | same, plus the production configuration |
| I5 | No word points into a fiber stack except the fiber's saved `rsp` (rebased by the runtime), `fs:[0x28]` while a shim runs, the old `rsp` pushed on the system stack by the §6.5 sequence during a kernel call, and the transient `r11` of the §6.4–6.5 sequences | machine-IR verifier | native tests with `--stack-initial 256` and with `--stack-move` |
| I6 | Rust never runs on a fiber stack | verifier: every call to a Rust symbol uses the §6.5 sequence or a shim | verifier tests |
| I7 | The kernel table matches declarations, ownership and layouts | none | kernel-use type test; per-kernel RC tests; `tests/layout.rs` |
| I8 | `nomi build` never resolves an `elm/*` package | project loader | a build test that imports an `elm/*` module fails with its diagnostic |
| I9 | Effects only through system kernels for `lib/` packages | frontend and build rules | diagnostic tests |
| I10 | Builds are deterministic | none | building twice in two different fresh temporary directories yields identical bytes |
| I11 | Object limits hold | compile-time checks; checked allocation asserts | layout tests |
| I12 | An object reachable from two fibers has rc < 0 or is immortal (M9) | `--rc-check` multi-core mode compares each touched object's owning fiber | stress tests, including sending a fresh slice of a buffer the sender keeps |
| I13 | No runtime lock is held across a switch or while generated code runs | per-worker lock count is 0 at every switch | scheduler tests |
| I14 | Every CFG cycle and tail-call target passes a poll; unchecked functions are loop-free, call only shims and have frames ≤ 256 bytes | machine-IR verifier | a fiber spinning in a tail loop must let another fiber run |
| I15 | No TCB-derived value is kept in a register across a call, poll, check or shim | LIR treats `fs:` loads as volatile; machine-IR verifier | M9 migration stress test |
| I16 | Fiber stacks never overflow below their limit | `--rc-check` canary at `limit − R` | all native tests |
| I17 | Equal keys hash equally | none | generated values, including override types built by different insertion orders and slicing paths: `a == b` implies `hash a == hash b` |
| I18 | No Rust runtime or `nomi-algo` code recurses to a depth set by its input | none | JSON nesting depth 10^6 and deep regexes run on an 8 MiB system stack |

## 9. Verification strategy

Every milestone extends the harnesses it inherits; none disables a check.

1. **Frontend parity (M2, M5).**
   - Overlays: every project gets its own directory under
     `~/.cache/nomi-dev/parity/projects/<id>/` with a generated `elm.json` and symlinks to
     its source directories. Each harness worker has its own `ELM_HOME` holding a copy of
     the frozen registry `~/.cache/nomi-dev/oracles/elm-0.19.3/registry.dat` (copied once
     from `~/.elm/0.19.3/packages/registry.dat`, 136,360 bytes); the oracle runs under
     `unshare -rn` without network, and any fetch attempt fails the harness; per package it
     holds a real directory with symlinks only to the corpus's `elm.json`, `LICENSE` and
     `src/`, never to `docs.json` or `artifacts.dat`; before each oracle run the harness
     asserts that no overlay symlink points at a file the oracle writes. Dependencies are
     prebuilt serially in dependency order. After every run the harness checks the corpus
     against a manifest of file count and mtimes. `XDG_CACHE_HOME` is writable.
   - Projects: every corpus package with no module importing `Elm.Kernel.*` becomes an
     application with `source-directories` = its own `src`, `elm/json` added,
     dependencies at their corpus versions transitively. Packages with such modules
     (`elm/*`, `elm/html`, `elm/svg`, `elm-explorations/*`) are checked as dependencies of
     a generated application: their exposed modules have verdict "accepted", and types
     come from `elm repl` with `import M`. Every application `elm.json` under `apps/`
     (outside `elm-stuff/` and vendored directories) is a project with `elm-version`
     rewritten to `0.19.3`; package-type ones are handled like corpus packages. Projects
     with dependencies missing from the corpus, or rejected before any module compiles, go
     to `tests/parity/excluded.txt` with the oracle's message.
   - Verdicts: one oracle run per project with `--report=json`; a module with a reported
     error is rejected; a module importing a rejected module, directly or transitively,
     has no verdict; every other module is accepted.
   - Types: per exposed top-level value, the repl's printed type (ANSI stripped) is
     parsed (row variables told apart from field labels), names resolved, aliases
     expanded, variables renamed in order of appearance keeping their constraint class,
     then compared with nomi's type normalized the same way. One repl session per project.
   - Oracle results live in `tests/parity/oracle/`, keyed by oracle version, corpus
     manifest hash, overlay-generator hash and registry hash; a mismatch fails;
     `cargo xtask parity --refresh` regenerates them.
2. **Mutation parity (M5).** A seeded mutator makes variants of accepted modules; verdicts
   must agree.
3. **Semantic tests (M1, M3).** Each test is a directory `tests/programs/<name>/` with
   `elm.json`, `src/Main.elm`, `expected.stdout`, `expected.stderr`, `expected.status`
   and an optional `flags` file (one of `native_only`, `needs_scheduler`,
   `order_sensitive`, `skip_naive`, `skip_stack_move` per line), written from `docs/semantics.md`, one or more per
   semantics rule. M1's tests run natively; from M3 every test runs under the reference
   interpreter and, from M4, every native configuration. Harnesses build and run in
   separate steps, so compiler and program exit statuses never mix.
4. **Program generator (M3).** Well-typed programs with custom types, records, closures,
   recursion, strings and the modules of §5.11a as they arrive; programs using
   `Process`, `Channel` or `Concurrent` are flagged `needs_scheduler`; 300 per `cargo test`, 10,000 per `cargo xtask fuzz`;
   `cargo xtask gen --seed S --lines N` writes a program of N real lines (about 40 bytes
   each) for the compile-time workload.
5. **Differential native testing (M4).** Reference interpreter; Core interpreter
   `--opt 0` and `2`; Flat interpreter `--opt 2`; native `--opt 0 --regalloc naive`,
   `--opt 1`, `--opt 2`, each with `--rc-check --stack-initial 256`; native `--opt 2` with
   `--force-yield`; native `--opt 2` without flags. Identical outputs, all runs with the
   same hash seed. M4 runs programs without the `needs_scheduler` flag; from M6 the
   flagged ones join, compared only between `--deterministic` native builds and the
   interpreter in virtual time (§9.9). Tests flagged `order_sensitive` (output depends on
   `HashMap` order) run with `NOMI_HASH_SEED=0` in every configuration, and their golden
   files are written for that seed; every other test runs under two different seeds and
   must produce the same output under both. `--heap-audit` joins the matrix at `--opt 2`.
6. **Stack and free stress (M1, M4).** `--stack-initial 256` everywhere; a native-only
   recursion of 10^7 frames (M1); dropping a 10^8-element list, a 10^6-element flat array
   of uniquely owned lists and a 10^7-node balanced tree on a 256-byte initial stack (M4);
   `--stack-move` on the whole M4 matrix once.
7. **Diagnostics snapshots (M2).** One per code.
8. **Fuzzing.** Parser (M2); JSON parser against a reference decoder in `nomi-algo` tests, with nesting depth up to 10^6 and objects with colliding keys
   under a time bound (M4); RESP protocol (M8).
9. **Deterministic scheduler (M6).** `--deterministic` builds and the reference
   interpreter share this state machine: no preemption; IO operations complete before the
   submitting fiber is re-enqueued at the tail (one operation in flight, taking no virtual
   time); the virtual clock starts at 0 and advances only when the run queue is empty,
   jumping to the earliest armed timer; all timers expiring at that instant form one tick
   and are enqueued in the order of a Fisher–Yates shuffle drawing from
   SplitMix64(`NOMI_SCHED_SEED`). Runtime tests run under 20 seeds.
10. **Runtime stress (M6, M9).** Preemption every microsecond with constant stack depth
    (stack sizes must not change); kill of a fiber parked on a pipe read that is never
    written (it must end within 200 ms, no freed access, zero leaks); handle generation
    preset near wrap; io_uring forced unavailable and a probe reporting a missing opcode
    (stdio still works, networking exits 103); 10^5 short connections leave the open-fd
    count flat; SIGTERM to an idle server exits within 1 second; `main` parked only on a `Signal` stream (no deadlock report); forced migration
    at every park (M9).
11. **`nomi-algo` against Rust's standard library (M3).** Float formatting against
    `format!("{:e}")` and round trip over 10^7 random and boundary bit patterns; number
    parsing against `str::parse`; case mapping against `char::to_uppercase`/`to_lowercase`
    on single-scalar mappings; from M7, the regex engine against a test-only backtracking
    matcher on generated patterns without backreferences.
12. **External oracles.** CommonMark spec examples (M7); `redis-cli` and named Valkey test
    files (M8, M10); `redis-benchmark` (M6); `openssl s_client` and `curl` (M11).
13. **Runtime archive freshness.** A test compares the embedded archive's hash with a
    fresh build of `nomi-rt`.

## 10. Performance workloads & targets

### 10.1 Workloads

| Workload | Sizes | Reference | From |
|---|---|---|---|
| Native data slice: tree build/fold, `List.range`/`map`/`foldl` | 10^5 | C programs in `bench/` | M1 |
| POC microbenchmarks in Elm syntax (fib, sum, collatz, evenodd, hof, tuples, ack, primes, tree) | POC sizes | the POC's C files, `gcc -O2` | M4 |
| Idiomatic: `List.map`/`foldl`, `Dict` insert/get, `HashMap` insert/get, string appends, `Array` set/get, JSON decode 10 MB with a 10^5-key object | 10^3, 10^5, 10^7 | C programs in `bench/` | M4 |
| Fibers: spawn, switch, channel ping-pong, 10^6 sleeping processes | — | Go programs in `bench/` | M6 |
| RESP-lite: PING/GET/SET, `redis-benchmark` pipelining 1/16, 1/50 clients; RSS for 10^6 small SETs | — | Valkey, single core | M6 |
| CommonMark | 1 KB, 100 KB, 10 MB (ASCII and mixed CJK), adversarial nesting | `cmark` | M7 |
| RESP full | `redis-benchmark`, `memtier_benchmark`, 1/50/1000 clients, pipelining 1/16 | Valkey, Dragonfly, redcon | M8–M10 |
| Compile time | each app; frontend over the parity set; 20,000-line generated programs | — | M2 |
| Binary size | every workload executable, stripped | — | M1 |

### 10.2 Targets

- Microbenchmarks ≤ 1.5× `gcc -O2`; idiomatic ≤ 3× the C reference.
- Fibers: spawn and switch at least as fast as Go. 10^6 sleeping processes within
  1.5 GiB resident (S5 measured 672 B and 2,208 B per fiber at 512 B and 2 KiB stacks; the
  1 KiB default class with a 128-byte record is estimated at about 1.2 KB).
- RESP-lite and RESP single-core: throughput ≥ Valkey, p99 ≤ Valkey's; 10^6 small SETs
  within 4× the payload plus keys. Multi-core ≥ 8× single-core on 12 cores, reported
  against Dragonfly.
- CommonMark ≤ 2× `cmark` at every size.
- Frontend ≥ 200,000 lines per second per core. `nomi build --opt 2` of 20,000 lines
  ≤ 2 s. Hello world ≤ 16 KiB.

### 10.3 Rule for a missed target

Recorded in the scoreboard with its profile; the milestone's `/optimize` phase works on it
before tagging. If still missed, it goes into the next milestone plan's Risks and the user
decides whether to tag.

### 10.4 Rules for compile time

- If `nomi build` misses its target with more than 30% in frontend work on unchanged
  packages, a milestone plan adds a per-package interface cache keyed by the hash of the
  package's sources, the keys of its direct dependencies' entries, the kernel table and
  the compiler build id; entries store names as strings and types relocatably.
- If more than 50% is in the backend, the next milestone plan emits functions in parallel
  into separate assembly files, assembled in parallel, keeping I10.
- If more than 40% is in Core and Flat optimization, the next milestone plan runs the
  simplifier per binding group in parallel and halves the inlining budget at `--opt 1`.

## 11. Decision Log

| # | Decision | Alternatives | Why | Status |
|---|---|---|---|---|
| D1 | Elm 0.19.3's syntax and type system | own syntax; Elm plus typeclasses | a fixed spec; the user chose it | LOCKED |
| D2 | Own textbook compiler, not a port | port elm/compiler | the user's experiment | LOCKED |
| D3 | Elm parity covers the compile and type-checking phase only; runtime behavior is nomi's own (§1.1) | output parity with Elm on Node | the user's rule | LOCKED |
| D4 | `Int` and `Bitwise` 64-bit, wrapping | JS doubles; 32-bit | speed, consistency | LOCKED |
| D5 | Division by zero crashes | return 0 | JS artifact | LOCKED |
| D6 | Strings UTF-8, counted in scalars | UTF-16 | JS artifact | LOCKED |
| D7 | Corpus packages are a harness only | packages as a goal | the user's rule | LOCKED |
| D8 | No user FFI; Rust kernels and rustls in the core system | in-house everything | the user's choice | LOCKED |
| D9 | Rust compiler; CommonMark and RESP stress tests | self-hosting | the user's choice | LOCKED |
| D10 | Direct-style Task on stackful growable fibers, structured scopes (§6.10), io_uring, processes owning state, Lean-style shared RC | interpreted Task; stackless | speed; agreed | LOCKED |
| D11 | Single core first, work stealing later | multi-core first | risk ordering | LOCKED |
| D12 | GNU `as` and `ld` | own encoder and linker | pragmatic | LOCKED |
| D13 | No syntax additions | do-notation | the syntax is Elm's | LOCKED |
| D14 | Header of §6.1 | POC header; two words | shared encoding and tag in one word | OPEN |
| D15 | Immediates `2k + 1`; constructor index rule of §6.2 | always boxed | no allocation for `Nothing`, `[]` | OPEN |
| D16 | Pointers-first fields | scan bitmap | one count describes every object | OPEN |
| D17 | Closure `info` pointer | inline tagged entry word | uniform rule; the inline form needs a new partial-application design | OPEN |
| D18 | TCB, checks, polls (§6.4) | guard pages; segmented stacks | memcpy growth needs a limit; preemption reuses it; cost: I5 rules out stack allocation of escaping-analysis candidates, so allocation removal relies on reuse, CPR and worker/wrapper | OPEN |
| D19 | Stack arguments, callee-pops tail calls, reserved `r10`/`r11` | POC static array | breaks with fibers | OPEN |
| D20 | Globals immortal after init through `nomi_globals` | RC on globals | no RC traffic on read-only data | OPEN |
| D21 | nomi ships its own standard library (`nomi/core`, `nomi/platform`) in Elm syntax, adopting elm/core code where it is a good foundation, with no compatibility obligation; nomi may add primitives | vendoring Elm's packages with parity | only syntax and type system are owed to Elm | LOCKED |
| D22 | Kernel table schema, classes, ownership, closed result set (§7.3) | untyped kernel calls | Perceus needs ownership; one layout authority | OPEN |
| D23 | Reference interpreter; algorithmic kernel cores shared through `nomi-algo`; Core and Flat interpreters ported | fully independent kernels; no Flat interpreter | one implementation of each algorithm; the native matrix checks the rest; Flat localizes RC bugs | OPEN |
| D24 | `nomi-rt` is its own workspace; `crates/nomi/build.rs` runs `cargo build --release --manifest-path $CARGO_MANIFEST_DIR/../nomi-rt/Cargo.toml --target-dir $CARGO_MANIFEST_DIR/../../target/rt` with `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_DIR`, `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER` and `CLIPPY_ARGS` cleared (verified: without the last three, `cargo clippy` runs clippy on the runtime and fails it), emits `rerun-if-changed` for `../nomi-rt/src`, `../nomi-rt/Cargo.toml`, `../nomi-algo/src`, `../nomi-algo/Cargo.toml` and `rerun-if-env-changed` for those variables, and embeds the archive | runtime next to the executable; whole workspace aborting | cargo forbids per-package `panic`; the compiler unwinds | OPEN |
| D25 | No Elm platform layer | Elm's architecture on the server | the user takes Elm's syntax and types, not its JavaScript runtime model | LOCKED |
| D26 | `main : Task Never ()`; returning ends the program with 0; `Exit.withCode` 0–100 | `main : Task String ()` | errors stay explicit; the user chose it | LOCKED |
| D27 | M1 is a native vertical slice exercising real data | parity first | proves every stage early | LOCKED |
| D28 | Rust only on system stacks; stackless scheduler | leaf kernels on fiber stacks | no stack reserve; the switch is 6 instructions | OPEN |
| D29 | No guard pages on fiber stacks; pooled regions; canaries under `--rc-check` | guard page per stack; 1 GiB reservations | 10^6 fibers would exceed `max_map_count`, and 10^6 × 1 GiB exceeds the 47-bit address space | OPEN |
| D30 | Kill and cancel at kernel boundaries; every crash exits 101; streams bound input | isolated crashes leaking memory; stack maps | no leaks, no extra format | LOCKED |
| D31 | Generation handles owned by the opening fiber | close on last reference | no fd reuse across connections | OPEN |
| D32 | String index and capacity (§6.2) | linear indexing; copying appends | O(1) amortized indexing and appends | OPEN |
| D33 | io_uring probed on first use; blocking stdio/`File` fallback; networking requires io_uring | probe at startup; full epoll fallback | CLI programs work where io_uring is blocked | LOCKED |
| D34 | Runtime semantics of §1.1 | Elm's JavaScript behavior | nomi's own platform | LOCKED |
| D35 | Linux-only CI | multi-OS | the target is Linux | OPEN |
| D36 | Free-list heads in the TCB; inline sequences of §6.1 | heap block pointer | one dependent load less per allocation and free | OPEN |
| D37 | State-owning processes as sharded channel loops, pinned in M9 | keyspace in one place; shared map | in-place updates and no contention | LOCKED |
| D38 | Milestone order of §14 | RESP first in M8 | measure the riskiest assumption early | LOCKED |
| D39 | Linear-time regex, ECMAScript syntax minus backreferences and lookaround | backtracking | no catastrophic backtracking on a server | LOCKED |
| D40 | `nomi/platform` modules: `Stdio`, `File`, `Tcp`, `Stream`, `Sink`, `Channel`, `Concurrent`, `Exit`, `Env`, `Signal`, `HashMap`, and `Tls`, `Http` (added 2026-11-06 with the user's agreement) | prefixed names | the user agreed | LOCKED |
| D41 | `HashMap` in `nomi/platform`, persistent, updated in place when uniquely owned, keys of any type `==` accepts, generated hash | `Dict` only | Valkey-class lookups | LOCKED |
| D42 | `HashMap` as CHAMP with inline pairs (§6.2); seeded SipHash-1-3 over the §6.13 word stream for hash indexes, `HashMap` and routing; iteration order follows the hash, fixed under a fixed seed | HAMT with entry objects; flat open-addressed table; unkeyed FxHash | one miss and about 40 bytes less per key than entry objects; snapshots share structure; resistance to hash flooding | OPEN |
| D43 | Exit drain under `--rc-check` (§6.9) | exempting live fibers' objects | the leak check stays exact | OPEN |
| D44 | Scheduling order of §6.10: spawn enqueues the child and the parent continues; immediate operations do not yield | Elm's order (child first, yield on every binding) | fewest switches; runtime behavior is nomi's own | OPEN |
| D45 | Rejected: small-string immediates, pointer tagging, inline closure entry, product flattening into cons cells | — | each adds a branch or masking to every RC operation or kernel, or multiplies layouts | OPEN |
| D46 | Polymorphic recursion through a binding group is rejected by `nomi build` (accepted by `nomi check`) | a boxed fallback representation | only checking parity is owed to Elm | LOCKED |
| D47 | Owned and sliced `Bytes` forms distinguished by tag; reserved runtime tags 248–254 | one sliced form | a stored small value costs one object, keeping the RESP memory target | OPEN |
| D48 | `rt_free` uses the POC's segmented pending stack | intrusive link through word 1 | word 1 holds a child in pointers-first objects | OPEN |
| D49 | Per-class free-list counts in the TCB with trimming to the span allocator | unbounded lists | memory returns across classes and to the OS | OPEN |
| D50 | Fiber registers saved on the fiber stack; fiber records about 128 bytes | registers in the record | 10^6 sleeping fibers within the S5 budget | OPEN |

## 12. Delegated decisions

| Decision | Owner | Constraints |
|---|---|---|
| Register assignment between generated functions | M1 | §6.6 rules |
| Refill policy behind `rt_alloc_slow`, span allocator and page bookkeeping | M1; M4 may replace | §6.1 sequences and D49 trimming; `size` is the exact word count; no syscall per allocation in steady state; spans come from shared mappings, never one mapping per object |
| Stack size classes and shrink policy | M1; M6 may replace | §6.4 geometry; fiber records per D50; shrinking happens only at a park, never below the live part plus R |
| Interpreter `Value` type | M3 | §5.10 signature; represents §6.8 lowered forms |
| Layout-rule grammar beyond the slice | M2 | corpus parity |
| `nomi/core` collection designs (ordered maps and sets, arrays) | M4 | §6.1–6.2; §5.6 overrides; node kinds through intrinsics and the header tag, never pointer tagging |
| Reuse report line format; `docs/scoreboard.md` entry format | M1 | one line per allocation site: position, outcome, reason; one scoreboard row per workload with date, commit and every §10 metric |
| `nomi check --machine` output (per-module verdicts and per-value types for the harness), `tests/parity/oracle/` file format, `tests/parity/excluded.txt` line format | M2 | types printed in the §9.1 normalized form; oracle cache keyed per §9.1 |
| `HashMap` operations and collision nodes | M4 | §6.2 CHAMP layout; I17; its hash projection is independent of node and collision order (a sorted list of per-entry hashes) |
| `Concurrent` scope kernels and result slots | M6 | system kernels named `rt_scope_*`; a slot is a tag-253 runtime record holding a status and a `boxed` payload; the spawn wrapper receives its slot as a capture; I13; the reference interpreter implements the same operations |
| JSON value constructors and key-index format (slot width, empty sentinel, probe sequence, load factor, duplicate handling) | M4 | shared by `layout.rs`; built by one leaf kernel; §6.13 hash; last duplicate wins |
| `nomi/platform` APIs | each milestone adding them | names per D40; effects through system kernels; streams bound per-connection input and the server bounds open connections; `Tcp.listen`'s stream transfers each connection to the fiber that receives it; error types built in Elm |
| io_uring ring sizes, provided buffers, low-ring copy fallback | M6 | §6.1 foreign-owned buffers; §6.6 slice copy rule; descriptors close through the ring after every SQE that names them |
| Channel implementation | M6 | §6.11; I13 |
| Timer storage | M6 | O(1) insert and cancel |
| `Time.now`, `Time.here` kernels (including `/etc/localtime`) | M6 | do not yield (§6.10) |
| Shared marking, cross-worker free, global quiescence for exit and deadlock | M9 | §6.6 rule; I12; size-class memory freed onto the freeing worker; foreign buffers return to their owner through an MPSC queue |
| Lazy free of large values | M10 | bounded work per scheduler tick |
| TLS crypto provider | M11 | no libc, `no_std` + `alloc`; the first choice is a pure-Rust provider (`rustls-rustcrypto`); if it does not link under `ld -static -nostdlib`, M11 ships HTTP without TLS and TLS moves to a later milestone |
| Regex engine | M7 | D39 |

## 13. Risks & mitigations

| Risk | Mitigation |
|---|---|
| The ported backend never compiled ADTs, strings, floats or tagged-union reuse | M1 exercises constructors, lists, reuse and frees end to end; the Flat interpreter localizes RC bugs; M4 completes the language with the §9.5 matrix |
| Uniqueness lost silently | reuse report; allocation counts on the scoreboard |
| Idiomatic Elm allocates heavily; boxed tree nodes | monomorphization, immediates, reuse, in-place arrays and strings; node kinds for collections (§12) |
| Compile time; IRC on huge functions | §10.4 rules; §5.8 IRC structures and threshold |
| Shared RC contention | immortal globals, borrow inference, pinned state-owning processes |
| Layout-rule corner cases | corpus and mutation parity |
| Leaf kernels cannot be preempted | O(input) rule; large inputs processed in chunks by Elm-source wrappers where the RESP p99 needs it |
| Retained `Bytes` slices pin buffers | the §6.6 copy rule; RSS target in RESP-lite |
| Scope growth and restarts | this document is the contract; every milestone ends with something running |

## 14. Milestones

### M1 — Native vertical slice
- **Goal:** one nomi program through every stage, with real heap data, on a growable
  fiber stack.
- **Works end to end:** `nomi build hello/` (`elm.json` lists `nomi/core` and
  `nomi/platform`): `main` builds `type Tree = Leaf | Node Tree Int Tree` to depth 20,
  maps it with `mapTree`, folds it, computes
  `List.foldl (+) 0 (List.map f (List.range 1 100000))`, and prints
  both with `Stdio.print`; the executable prints the numbers and exits 0.
- **Builds:** project loading; first `docs/grammar.md`; syntax, names, types (with
  instantiations and defaults) and patterns for the slice; Core with monomorphization,
  direct-style `Task` and `main`; Flat with Perceus and reuse; LIR; x86 with checks,
  polls, the §6.1/§6.5/§6.6 sequences and the verifier; `nomi-rt` (`_start`, TCB,
  `rt_enter`, main fiber, `rt_morestack`, shims, allocator, `rt_free`, globals, stdout,
  exit, `SIGPIPE`/`SIGSEGV`, checked shims and quarantine, reports); D24; `nomi_config`,
  `nomi_globals`, `nomi_sites`; the M1 modules of §5.11a; the first `docs/semantics.md`
  and the `tests/programs/` layout with the slice's tests;
  `Stdio.print`.
- **Acceptance:** zero leaks under `--rc-check --stack-initial 256`; `--stats` reports a
  nonzero allocation count; the reuse report shows reuse hits in `List.map` and `mapTree`; a
  native-only variant recursing 10^7 frames; the §6.8 `andThen` check; hello world
  stripped ≤ 16 KiB.
- **Measurable:** binary size, compile time, the native data slice workload.
- **Status:** planned

### M2 — Frontend parity on the shipped packages
- **Goal:** `nomi check` agrees with the oracle on Elm's own packages from the corpus
  (`elm/*`, checked as dependencies, §9.1); `elm-explorations/*` joins in M5.
- **Builds:** full syntax and grammar, names, types, patterns, diagnostics, parity overlays and oracle cache, parser fuzzing.
- **Acceptance:** 100% verdict and type agreement on that set; frontend ≥ 200,000 lines
  per second.
- **Status:** planned

### M3 — Reference semantics
- **Goal:** the reference interpreter implements `docs/semantics.md`.
- **Builds:** `nomi interp`, `nomi-algo`, reference kernels for the M3 modules of §5.11a,
  `docs/semantics.md` for them, the semantic suite, the program generator.
- **Acceptance:** every semantic test passes; 10,000 generated programs run without
  interpreter errors; `nomi-algo` float, number and case-mapping tests (§9.11) pass.
- **Measurable:** interpreter throughput on the semantic suite.
- **Status:** planned

### M4 — Native backend for the whole language
- **Goal:** every M3 program without the `needs_scheduler` flag runs natively with
  identical output.
- **Builds:** all §6.2 layouts, closures, Float, strings with index and capacity,
  flat arrays, `HashMap`, records with reuse, generated equality/comparison/hash/toString,
  leaf kernels with `layout.rs`, the `nomi/core` collections, bytes and JSON with its
  fuzzer, reuse report, `--stats`, Core and Flat interpreters, the §9.5 matrix and §9.6
  stress tests.
- **Acceptance:** differential tests green; microbenchmark and idiomatic targets met or
  handled by §10.3.
- **Status:** planned

### M5 — Parity on the whole corpus
- **Goal:** `cargo xtask parity` agrees on every corpus and app project with verdicts;
  mutation parity.
- **Acceptance:** 100% agreement; 10,000 mutants agree; `excluded.txt` reviewed.
- **Status:** planned

### M6 — Fiber runtime, single core, and RESP-lite
- **Goal:** concurrent nomi programs on one core, measured against Go and Valkey early.
- **Builds:** scheduler with §6.10 order, scopes and cancellation (natively and in the
  reference interpreter), `Process.spawn`/`kill`/
  `sleep`, channels, state-owning processes, `Time.now`/`Time.here`, io_uring files,
  stdio and TCP with provided buffers and the blocking fallback, timers, monitor thread,
  signals, `Concurrent`, `Exit`, `Env`, `Signal`, deterministic mode, runtime stress
  tests; `apps/resp` with PING, GET and SET.
- **Acceptance:** runtime tests under 20 seeds; `needs_scheduler` programs in the §9.5
  matrix; I14 tight-loop test; `redis-benchmark` against Valkey; fiber and RESP-lite
  targets of §10.2 met or handled by §10.3.
- **Status:** planned

### M7 — CommonMark
- **Goal:** `apps/commonmark` renders CommonMark to HTML; regular expressions ship in
  `nomi/core`.
- **Acceptance:** all spec examples pass; ≤ 2× `cmark` at every size.
- **Status:** planned

### M8 — RESP v1
- **Goal:** strings and expiry with `Stream` and `Sink`, zero-copy `Bytes`.
- **Acceptance:** `redis-cli` scripts; Valkey's `unit/type/string.tcl` and
  `unit/expire.tcl` minus tests using `DEBUG`, `OBJECT`, `CONFIG` or encodings (the M8
  plan lists the remaining names); RESP fuzzer; single-core targets met or handled by
  §10.3.
- **Status:** planned

### M9 — Multi-core
- **Goal:** one worker per core with work stealing of unpinned fibers.
- **Builds:** Chase–Lev deques, global queue, shared marking with atomic RC (I12),
  per-worker io_uring, cross-worker free, global quiescence.
- **Acceptance:** scaling target of §10.2; I12 and migration stress tests.
- **Status:** planned

### M10 — RESP complete
- **Goal:** lists, hashes, sets, sorted sets, pub/sub, transactions, RDB/AOF persistence,
  shutdown on `SIGTERM`.
- **Acceptance:** the corresponding Valkey test files (listed in the M10 plan);
  crash-recovery tests.
- **Status:** planned

### M11 — TLS and HTTP
- **Goal:** TLS 1.3 through rustls and an HTTP/1.1 server in `nomi/platform`.
- **Acceptance:** interop with `openssl s_client` and `curl`; HTTP benchmark recorded.
- **Status:** planned

## 15. Review history

### Round 1
Five critics. Changes: crashes exit 101, kill and cancel at system kernels, fiber-owned
resources, generation handles; Rust on system stacks; polls on back-edges and tail
calls; signal policy; deadlock exit; no guard pages on fiber stacks; scheduling order;
Cmd/Sub without map nodes; kernel ownership; JsArray length from the header; string
index and capacity; foreign-owned buffers; TCB allocation fields; slow-path shims;
polymorphic recursion and more §1.1 rows; production configuration in the matrix;
runtime embedded by `build.rs`; parity overlays (the oracle had written into the corpus
during a spike; the files were removed); global `DefId`; mono without caps. Rejected:
dropping the typed-AST reference interpreter (it catches frontend and monomorphization
bugs Node cannot).

### Round 2
Five critics. Changes: Elm's real step order (subscriptions before dispatch, immediate
delivery to the main fiber), verified in elm/core's `Platform.js`; the run-at-spawn
shortcut removed; stackless scheduler and `rt_enter`; shims save into a TCB slot and the
system stack; fiber contexts saved in fiber records; R = 512 with an exact budget,
`rsp − F` checks, verified unchecked functions, `--rc-check` canaries (I16); preemption
protocol without spurious growth; TCB loads volatile (I15); `PENDING` = 3; String layout
fixed (index first, only for capacity ≥ 256); string index never written into immortal or
shared objects; foreign buffers get a reserved tag 254; RC overflow via `jo`; generation
wrap retires slots; inline free hands children to the iterative worklist; in-flight
io_uring operations own their buffers and cancellation waits for completion; closed set
of Rust-built compound values with a layout test; `Program` value and generated worker
loop; `nomi_config` and `--stats`; JSON objects with a hash index; Json booleans
nullary; free-list heads in the TCB; state-owning processes and RESP sharding; RESP-lite
in M6; corpus-wide and mutation parity in M5; kernel packages checked as dependencies
(the oracle rejects kernel modules in source directories); per-project overlays,
per-worker `ELM_HOME`, corpus manifest check; per-module verdict rule; alias expansion in
the type comparison; `nomi-rt` as its own workspace with `rerun-if-changed`; §1.1 rows
for `Bytes` and Json equality, Json `Debug.toString`, multi-scalar case mapping; corpus
counts restated for packages' own `src/`. Rejected: running `update` on the finishing
task's fiber (would break the model's rc = 1 ownership; immediate delivery gives the same
order); dropping D17's alternative branch instead of specifying it (the alternative is
removed).

### User answers and spikes (2026-11-06)
The user accepted every recommendation: linear-time regex (D39), ECMAScript float
format, `Debug.log` to stderr in Elm's format, crashes exit 101 with bounded stream input
(D30), `Platform.worker` with `()` flags, `nomi/platform` names (D40), all §1.1 rows
(D34), the milestone order (D27, D38), io_uring probed on first use (D33), `HashMap`
(D41), pinned state-owning processes (D37). Spikes, run in `~/.cache/nomi-dev/spikes/`
and deleted: S1 — Elm computes subscriptions first and runs a step's commands in reverse
flattened order, each `update` running before the next task, new commands at the tail
(§6.10 changed); S3 — repl prints aliases by name with module qualification (§9.1); S4 —
about 50 ms per repl query in one session; S5 — 672 B per sleeping fiber with 512 B
stacks, 2,208 B with 2 KiB, 4,256 B with 8 KiB, from one pooled mapping (§6.4, §10.2).

### Platform layer removed (2026-11-06)
The user takes Elm's syntax and type system, not its platform layer. Removed
`Platform.worker`, `Cmd`/`Sub`, effect managers, the generated worker loop and the step
order; `main : Task Never ()` (D25, D26). Spike S1's step-order result is no longer used.

### Round 3
Five critics on the plan without the platform layer. Changes: Elm's real scheduler order
(spawn runs the child first; kill drops queued fibers; `Time.now`/`Time.here` do not
yield, D44), verified on the oracle; structured scopes defined (§6.10); channels as
system kernels replacing mailboxes; exit drain under `--rc-check` (D43, exit 104);
deadlock rule counts `Signal` waits; blocking stdio/`File` fallback without io_uring;
seeded SipHash for hash indexes and `HashMap` (D42); generated hash family and I17;
`HashMap` as a HAMT with single-object nodes; constructor index rule and encodings;
inline allocation and free sequences, shim argument registers, reserved `r10`/`r11`,
`rt_morestack` saving all GPRs and XMM registers, `--force-yield` and `--stack-move`;
`nomi_globals` and `rt_immortalize_globals`; `nomi_config` offsets and `deterministic`;
crossing rule marks every rc > 1 subgraph and copies small or foreign `Bytes` slices;
intrusive free worklist and large-drop tests; foreign buffers always large; cancel waits
at most 100 ms; stack geometry with limits inside the class; `Typed` carries
instantiations and per-constraint defaults; kernel table schema; JSON `Value` exact
constructor order and hash index type; overlays symlink only `elm.json`, `LICENSE`,
`src/`; Flat interpreter kept and `nomi-algo` shared kernel cores (D23); IRC data
structures replaced; M1 exercises real heap data; D24 paths from `CARGO_MANIFEST_DIR`;
`layout.rs` shared by `#[path]`; reserved exit statuses; static data in `.rodata`.
Rejected: a native data milestone before M2 (M1 now exercises constructors, lists and
reuse, and M4 follows M2–M3 as locked in D38); pointer tagging, small-string immediates
and inline closure entries (D45).

### Runtime parity removed (2026-11-06)
The user: nomi matches Elm's compile and type-checking phase, not its runtime behavior.
Removed output parity against Elm on Node; §1.1 became nomi's own runtime semantics in
`docs/semantics.md`; the scheduler no longer mimics Elm (spawn: parent continues;
immediate operations do not yield, D44); semantic tests use golden files; `nomi-algo`
shares all kernel cores. The `HashMap` seed question is settled by D42 (iteration order
unspecified).

### Elm's standard library optional (2026-11-06)
The user: only Elm's syntax and type system are owed to Elm; the stdlib may be adopted
where it is a good foundation, primitives may differ. Removed vendoring with parity and
`PATCHES.md`; `nomi/core` starts from elm/core's pure Elm modules and changes freely
(D21); `nomi build` never resolves `elm/*` (I8).

### Round 4
Five critics. Changes: M4 covers programs without the `needs_scheduler` flag and M6 adds
the rest under deterministic comparison with a specified tie-break; hash composition
fixed and shared through `nomi-algo`, seed in `nomi_hash_seed`; `rt_free` back to the
POC's segmented pending stack (D48); reserved tags 248–254 so runtime walks recognize
strings, bytes and arrays; absent string index = immediate 1; `scan(p)`/`size(p)` for
large objects; owned and sliced `Bytes` (D47); `HashMap` as CHAMP (D42); imm64 header
store; per-class free-list counts and trimming (D49); `--heap-audit` over the production
inline paths; site ids through `fs:[0x40]` and `nomi_sites`; `nomi_globals` lists only
pointer slots; registers saved on the fiber stack, small fiber records (D50); growth to
the class ≥ used + F + R; `info.entry` convention with raw scalar words; override rows in
the kernel table; interpreter kernels may return `Pending`; `nomi-algo` without `alloc`;
global `ExprId` with CSR instantiations; `lib/` embedded; prelude format; system stacks
8 MiB and I18; io_uring opcode probe; monitor wakes idle workers and enforces deadlines;
handles process-wide with owner checks; registry frozen and the oracle offline; third
compile-time rule for the optimizer and a defined generator; M1 reuse checked on
`List.map`/`mapTree`; polymorphic recursion recorded as D46; `nomi-algo` tested against
Rust's standard library. Rejected: dropping the typed-AST reference interpreter (it
checks `lower_core` and monomorphization independently); a single-fiber scheduler in M4
(M6 adds concurrent programs instead).

### Final DRY-RUN 1
Changes: unchecked functions are loop-free so no poll runs below them; the stack canary
moved below R into each class's lowest 16 bytes; `NOMI_HASH_SEED` for every build and
the interpreter; `nomi_config` gained `stack_move`, `force_yield`, `heap_audit` with
offsets; reports go to `NOMI_REPORT_PATH`, exit 105 on leak; the hash stream as a table;
the startup contract (`w_main` calls `w_init` and the immortalization walk; flagged main
fiber in `rt_fiber_exit`); module introduction table (§5.11a) with M3 scoped to its
modules and the interpreter scheduler built in M6; deterministic state machine and channel
wake and close rules; the string index format; the CLI synopsis; JSON value format
delegated to M4; verified that an excluded `no_std` runtime workspace can path-depend on a
workspace member through the D24 nested build.

### Final DRY-RUN 2
Changes: the canary counted inside R (default class 1 KiB); `Stream`'s pull core moved to
M6; channel send ends are shareable capabilities; spawned closures are compiler wrappers
of type `Task Never ()` with runtime result slots and a state-owning flag; checked,
stats, audit and deterministic support linked only through init calls from `w_main`,
`nomi_sites` always emitted; `--heap-audit` counter instructions and slots; system kernels
lower to task closures; `Exit.withCode` cancels every fiber including the caller and
exits after the drain; semantic-test directory layout and flags; `Random` in M3; M11's
`Tls`/`Http` modules pending the user's agreement (D40 is LOCKED).

### Final DRY-RUN 3
Changes: `String.fromInt`/`++` and the small `String` layout with one leaf kernel in M1;
override function types (three families; hash as a canonical projection); the `boxed`
kernel kind and box layout for values crossing fibers; the canary fixed at `limit − R`;
`rt_alloc_checked`/`rt_free_checked` contracts; harness and scoreboard formats delegated
to M2 and M1; tag 253 defined; sized integers no longer named before a milestone adds
them. These fixes were not re-checked: the review loop's limit of three final DRY-RUN
critics was reached.

### User answers (2026-11-06, after final DRY-RUN 3)
The user agreed to extend D40 with `Tls` and `Http`, and to one more DRY-RUN critic beyond
the loop's limit to verify the last fixes.

### Extra DRY-RUN (after the user's approval)
Four blockers fixed: override rows for three families on `nomi/*` types, keyed by names
and resolved by `lower_core`; no default imports inside the prelude's own package;
`Concurrent` scope kernels and result slots delegated to M6 with constraints;
`order_sensitive` tests fixed at `NOMI_HASH_SEED=0`, all others run under two seeds. Also:
one flag spelling, init calls before `w_init`, M1 builds both `String` layouts and the
first `docs/semantics.md`.

The user accepted the plan after the extra DRY-RUN fixes (2026-11-06).

blueprint final

### M1 plan amendments (2026-11-06)
While planning M1: D24 also clears `RUSTC_WRAPPER`, `RUSTC_WORKSPACE_WRAPPER` and
`CLIPPY_ARGS` (verified with a scratch workspace); `lower_flat` takes the diagnostics sink
(I11 errors); the machine-IR verifier runs after register allocation; the IRC
data-structure rewrite moves to M4; closure info objects use tag 251. Free-list trimming
(D49) needs scheduler switches and arrives with them (M6) and page return (M4); M1 keeps
the counts.
Also: the freed-field poison is `0xDEAD_DEAD_DEAD_DEAC`; `lower_lir` takes a `LirConfig`;
status 141 means stdout was closed (`EPIPE`); `nomi_hash_seed` starts in M4 with its first
reader; per-file spans are `TextRange` with `line_starts` (A§6.15's `Span` is used across
files). Round 3 of the M1 plan: `Diagnostic` gains a primary label and notes (§7.2); the
parser keeps operator chains flat and `ast/` resolves fixity (§5.2).
Round 4 of the M1 plan: D24's nested build sets `CARGO_ENCODED_RUSTFLAGS` to the empty
string, removes every `CARGO_BUILD_*`/`CARGO_PROFILE_*` variable, passes `--target
x86_64-unknown-linux-gnu` and `--config build.rustc-wrapper=''` (verified against a
config file that sets `build.rustflags` and a wrapper); Flat CPR moves to M4 with the
other deferred passes; `Stdio.print : String -> Task x ()` is a system kernel with an
unconstrained, never-produced error variable (an exception to §7.3's monomorphic
signatures); `nomi_algo::fmt_i64` writes into a fixed 20-byte buffer instead of an output
trait (fixed-size result); E0304 admits only `nomi/*` in M1, since E0103 rejects `elm/*`
dependencies until M2; `AllocSeq` may target any register, and `IncSeq` adds n with one
`add`.
Final dry-run of the M1 plan: the shim list (§6.6) gains `rt_saturate_checked` (checked
builds; makes a saturated object immortal and settles its site counter); the checked
report prints the runtime entry's leak line as `leak <runtime> <count>`; `rt_crash` takes
`(msg: *const u8, len: usize)`.
