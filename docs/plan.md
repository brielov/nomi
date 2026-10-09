# nomi build plan

Status: 2026-10-06. Companion to [architecture.md](architecture.md). Progress is tracked by the checkboxes below and `git log`, nothing else.

## Scope

One binary, `nomi`, that replaces three tools and then adds capabilities on top:

| Milestone | Replaces or adds | Parity target | Release |
|---|---|---|---|
| M2 | elm-format | 0.8.8: byte-identical output, same CLI | 0.1 |
| M8 | `elm` (make, init, install, bump, diff, publish) | 0.19.1, 0.19.2 and 0.19.3, chosen by `elm-version` | 0.2 |
| M9 | elm-language-server | 2.10.0 features, including its custom requests | 0.3 |
| M10 | elm-test | 0.19.3-0 flags and output | 0.4 |
| M11 | Server platform | Node kernels, `nomi/*` packages, fiber runtime | 0.5 |
| M12 | Workspaces | one resolution; packages shared by web and api | 1.0 |

The frontend-plus-backend goal is reachable at M11: two projects can already share code through `source-directories` (R7). M12 makes sharing first-class.

**Not planned:** `elm repl`, `elm reactor`, and elm-format's `--elm-version 0.18` input. Under the `elm` and `elm-format` names these exit with a one-line explanation.

## Working rules

1. **One step, one commit.** The step's title is the commit subject. The commit body names any salvaged source (`brielov/nomi@8e2b540:<path>`) and anything surprising. The same commit ticks the step's box.
2. **Green at every commit.** `cargo xtask ci` passes. Write a step's check before its code. Nothing is `#[ignore]`d; unfinished parity is tracked by ratchets.
3. **Small steps.** If a step grows past about 1,000 hand-written lines (fixtures and snapshots don't count) or one working session, split it here first, in its own commit: `plan: split N.M`.
4. **Rolling detail.** Near milestones are detailed. Later ones get refined when they start (`plan: refine MN`). Exit criteria change only with a `DECISIONS.md` entry.
5. **Gates.** A milestone ends when its exit criteria hold on the tier-2 suites. You review at each gate. Releases are tagged locally. Nothing is pushed or published without your go-ahead.
6. **Fix representations, not symptoms.** If the same class of mismatch needs a third heuristic, the step stops, and the representation changes with a `DECISIONS.md` entry. The old formatter took about 35 single-mismatch rounds, all landing in one file.
7. **Small files.** A file past about 1,500 lines gets split by responsibility; `xtask ci` enforces it.
8. **Documents.** There are only four: this plan, `architecture.md`, `README.md`, and `DECISIONS.md` (one dated paragraph per decision that would be expensive to reverse). Benchmark numbers go in `docs/benchmarks.md`. If a decision can be a test, write the test. There are no handoff or status files.
9. **Use established crates** where one fits (clap, rowan, pubgrub, lsp-server, serde, insta, proptest and the like) rather than writing our own. The exceptions are where exact upstream behavior requires it, and pure crates still take only pure dependencies.
10. **Never** write upstream's files (`elm-stuff/0.19.x`, `~/.elm`) from nomi. Oracles run in sandboxes with their own `ELM_HOME`. The development tooling downloads only what `references.toml` or `corpus/` pins by hash.

**Sessions.** Work happens in sessions that don't share memory, so each one:

1. starts at the first unchecked box;
2. writes the check, implements, and runs `cargo xtask ci`;
3. ticks the box and commits;
4. stops to ask when a step needs a decision, a download or a push.

**Picking up on another machine.**

1. Clone the `rewrite` branch of `brielov/nomi` (`master` there is the old repository).
2. `cargo xtask refs` syncs the gitignored `reference/` sources from `references.toml`; the oracle sandboxes and corpus are fetched by the xtask commands that need them, pinned by hash.
3. `cargo xtask ci` must pass before the first edit.
4. Resume at the first unchecked box. Right now that is **5.1**.

### Test tiers

| Tier | Command | Needs | Runs |
|---|---|---|---|
| 1 | `cargo xtask ci` | Rust, plus Node from M8 | every commit |
| 2 | `cargo xtask corpus <suite>` | the corpus cache, fetched once and pinned by hash | steps that touch a suite, every gate, CI on push |
| 3 | `cargo xtask oracle <suite>` | pinned upstream binaries | regenerating snapshots; new upstream releases |

**Ratchets.** Each parity suite stores the oracle's results, as hashes when they're large, plus a sorted `passing.txt`.

- A listed case that fails is a regression and fails CI.
- A case that newly passes is reported until it's added.
- A milestone that claims parity needs every case listed, apart from deviations recorded in `DECISIONS.md`.

Tests that need Node fail loudly when it's missing; they never skip.

## Decisions (settled 2026-10-07)

- **D1 License.** BSD-3-Clause, like Elm and elm-format. Ported code keeps upstream's notices in `THIRD-PARTY-NOTICES.md`: Elm and elm-format are BSD-3-Clause, elm-language-server is MIT.
- **D2 Repository.** `nomi/` is its own repository, on `master`. Where it gets pushed is decided later.
- **D3 Downloads.** Approved when pinned by hash:
  - step 0.6: elm 0.19.1, 0.19.2 and 0.19.3 and elm-format 0.8.8 from GitHub releases, and elm-test and elm-language-server from npm with `--ignore-scripts`;
  - step 0.7: every registry package at its latest version, the four apps, and the JDK 21 and sbt that docspell's code generation needs once;
  - step 3.6: GHC 9.8 for the solver oracle, if 9.14.1 can't build it.
- **D4 Scope.** No REPL, no `elm reactor`, no elm-format `--elm-version 0.18`. `nomi run --watch` over workspace members (12.7) takes the reactor's place.

## M0: Repository and harness

Goal: an empty workspace whose rules are enforced by tooling, with oracles and corpora reproducible from pins.

- [x] **0.1** `repo: start nomi as its own repository` (D2). Run `git init -b master` and add a `.gitignore` for `target/`, `reference/` and `.DS_Store`. Commit `docs/`, a one-paragraph `README.md`, an empty `DECISIONS.md`, and the current `Cargo.toml` and `src/`. **Check:** `git status` is clean.
- [x] **0.2** `repo: add license and third-party notices` (D1). **Check:** both files exist, with upstream copyright lines verbatim.
- [x] **0.3** `build: create the cargo workspace with base, syntax and cli`. A virtual workspace with `crates/{base,syntax,cli}`, packaged as `nomi-*`. The binary is `nomi`, with `--version`. Workspace lints include `unsafe_code = "forbid"`. **Check:** `cargo run -p nomi-cli -- --version`.
- [x] **0.3a** `cli: parse arguments with clap`. Subcommands are added by the steps that implement them; each one lists its Elm-name aliases (architecture §9).
- [x] **0.4** `xtask: add the ci command and architecture checks`. `cargo xtask ci` runs fmt, clippy (`-D warnings`) and the tests, plus four checks:
  - crate edges are a subset of the allowed graph in architecture §4;
  - pure crates use no filesystem, network, process, thread, environment or clock APIs, and depend on no crate that does;
  - no hand-written source file exceeds 1,500 lines;
  - there is no `#[ignore]`.

  **Check:** each check has a test with a failing fixture.
- [x] **0.5** `ci: run xtask ci on Linux, macOS and Windows`. GitHub Actions; it stays dormant until the repo is pushed.
- [x] **0.6** `xtask: pin references and fetch oracles` (D3).
  - `references.toml` pins each of these by tag, commit and archive SHA-256: elm 0.19.1, 0.19.2 and 0.19.3, elm-format 0.8.8, elm-language-server 2.10.0, elm-test 0.19.3-0 and elm-explorations/test. Node comes from `PATH` and must be at least the oldest maintained LTS.
  - `xtask refs` syncs `reference/`.
  - `xtask oracle fetch` installs the binaries under `~/.cache/nomi-dev/`.

  **Check:** both are idempotent, and `xtask oracle versions` prints every pin.
- [x] **0.7** `xtask: pin and fetch the corpus` (D3).
  - `corpus/packages.toml` lists every registry package at its latest version, with the hash from its `endpoint.json`.
  - `corpus/apps.toml` pins exosphere, liascript, docspell and elm-3d-scene by commit, taken from the old browser manifest, together with docspell's code-generation step and the hash of its output.
  - Sources live in `~/.cache/nomi-dev/corpus/` and never enter the repo.

  **Check:** a refetch verifies every hash, and `xtask corpus stats` prints counts.
- [x] **0.8** `xtask: add the parity ratchet runner`. It covers suites, expected results, `passing.txt`, regression and newly-passing reports, and `xtask ratchet <suite>`. **Check:** a self-test on a toy suite.

**Exit:** a fresh clone passes `cargo xtask ci`, and refs, oracles and corpus rebuild from pins alone.

## Lane R: fiber runtime prototype (any time after M0)

This is plain JS in `runtime/` with no npm dependencies, tested with `node --test` and an in-repo seeded model checker. It needs no compiler, so these steps fit wherever compiler work is waiting on something.

- [x] **R.1** `runtime: add a seeded model-based test harness`. It runs random operation sequences against a reference model, and failing seeds print and replay.
- [x] **R.2** `runtime: fibers, exits and the run queue`.
  - Fork, join and await.
  - Exits are Success, Fail, Defect or Interrupt; a JS exception becomes a defect.
  - A fairness yield every N steps.
  - Finished fibers leave every table.
- [x] **R.3** `runtime: scopes and finalizers`. Closing a scope interrupts its children, then runs finalizers in LIFO order. Acquire is uninterruptible, and release runs exactly once.
- [x] **R.4** `runtime: interruption`. It is delivered at async boundaries and yield points, and respects masks. Host calls get an `AbortSignal`.
- [x] **R.5** `runtime: virtual clock`. It advances only when every live fiber waits on a timer, never while I/O is pending.
- [x] **R.6** `runtime: process lifecycle`. SIGINT or SIGTERM interrupts the root, finalizers run, the process exits with 130 or 143, and in-flight work drains within a deadline. Tested in child processes.

**Exit:** 10,000 seeds per property run clean, and every row of the runtime table in architecture §8 has a passing test.

## M1: Syntax

Goal: one lossless, tolerant parser used by every consumer, which accepts exactly what each upstream tool accepts.

- [x] **1.1** `base: text ranges, FileId and LineIndex`. Converts between bytes, (line, column) and UTF-16. The column unit stays a parameter until 1.17 settles it. **Check:** property tests round-trip.
- [x] **1.2** `base: diagnostic model`. Code, title, severity, spans, message document and fixes; no renderer yet. **Check:** unit tests.
- [x] **1.3** `syntax: token and node kinds`. One kind per distinct token, the rowan language, and a first `syntax/elm.ungram`. **Check:** kinds round-trip.
- [x] **1.4** `syntax: lexer`. Salvaged from the old lexer. It covers:
  - identifiers, operators and numbers;
  - nested and doc comments;
  - single-line and triple-quoted strings with escapes, and chars;
  - GLSL blocks;
  - error tokens for unterminated literals.

  There is no formatting mode. **Check:** the old lexer tests, plus a property that tokens concatenate back to the input.
- [x] **1.5** `syntax: event parser and tree builder`.
  - Markers, events, and Missing/Error nodes.
  - The salvaged event-to-rowan builder, with its losslessness assertion.
  - A depth guard plus `stacker::maybe_grow`.
  - Lazy trivia: leading comments go to the next node, and trailing trivia bubbles to the parent.

  **Check:** builder tests; synthetic recursion at depth 100,000 doesn't abort.
- [x] **1.6** `syntax: module header, imports and exposing`, including `port module` and `effect module … where { … }`. **Check:** tree snapshot fixtures, as for every grammar step.
- [x] **1.7** `syntax: types`.
- [x] **1.8** `syntax: patterns`.
- [x] **1.9** `syntax: atoms and operator chains`.
  - Literals, variables and qualified names.
  - Records, record update, field access and accessors.
  - Tuples, lists, unit and parentheses.
  - Operator functions and negation.

  Chains stay flat.
- [x] **1.10** `syntax: if, case, let and lambda`. Follows upstream's layout rules (fresh line, indented, aligned). Contextual splits such as `=-` happen in place and resynchronize. **Check:** fixtures; the token-split worst case parses in linear time.
- [x] **1.11** `syntax: declarations and doc comments`. Values with annotations, aliases, custom types, ports and infix declarations. A doc comment belongs to its declaration.
- [x] **1.12** `syntax: module-level recovery`. Resynchronizes at the next column-0 token that can start a declaration, with one error per recovery. **Check:** the "comment about `y` stays out of `x`" fixture and the stray-token fixture.
- [x] **1.13** `syntax: generated typed views`. Generated by `xtask codegen` from `elm.ungram`; `xtask ci` fails if the output is stale. **Check:** every node kind has a view.
- [x] **1.14** `syntax: add declarations and expressions entry points`. elm-format uses these for Elm code blocks in doc comments.
- [x] **1.15** `syntax: validation pass`. Flags what the grammar accepts but a tool rejects, keyed by context: application, package or kernel source, and elm or elm-format.
- [x] **1.15a** `corpus: expectations for each Compat`. Upstream accepts an application only when its `elm-version` equals the compiler's (`Elm/Details.hs`, `verifyApp`), and a package only when its range admits the compiler. The pinned apps all say 0.19.1. Of the pinned packages, 15 exclude 0.19.3, 13 exclude 0.19.2, and 2 exclude even 0.19.1 (one range ends at 0.19.0; one tag has no `elm.json`). `corpus/compat.toml` records them. So:
  - oracle sandboxes rewrite each app's `elm-version` to the Compat under test, and the rewrite is part of the suite's pins;
  - `xtask corpus` classifies each package by the Compats its range admits;
  - a project a Compat rejects is an expected-failure case whose diagnostic must match, not a gap.

  Every later "every corpus package/app/project" claim means "every one the Compat accepts, plus the expected failures". **Check:** `xtask corpus stats` prints the per-Compat counts; the classification is snapshotted.
- [x] **1.16a** `testkit: source mutations`. Port the old deterministic mutation generator: every insertion, deletion and duplication at sampled boundaries, plus seeded random combinations. Over the suite's eight seeds (three from the first attempt, five elm-format test files), it makes 39,314 cases. The old 8,617 came from a smaller generator that the current seeds cannot reproduce. **Check:** the generator is deterministic and pinned by tests.
- [x] **1.16b** `oracle: syntax accept/reject snapshots`.
  - Tier 3 runs elm 0.19.1, 0.19.2 and 0.19.3 (syntax errors only, from `--report=json`) and `elm-format --stdin` on the corpus and the mutations. elm runs in synthetic offline sandboxes, one per Compat, each with an application, a package and a kernel project and the module beside their `elm.json`. Without the sandboxes, 0.19.2 and 0.19.3 stop at `elm.json` before parsing.
  - The mutation verdicts go in `crates/syntax/tests/syntax_mutations/oracle/`, a tier-1 ratchet. The corpus verdicts go in `corpus/syntax/oracle.tsv` (only rows some tool rejects, plus a digest of every module), a tier-2 ratchet with a case per project: `cargo xtask corpus syntax`.
  - Surprises:
    - 0.19.2's JSON regions are 0-based (its message snippets are 1-based);
    - elm copies raw tabs and carriage returns into its JSON;
    - 0.19.3 crashes on some recursive type aliases; that is stored as `crash` and skipped;
    - elm very rarely hangs or trips over its own `d.dat` lock, so runs are timed out and retried.

  **Check:** snapshots are checked in and the ratchet exists.
- [x] **1.17a** `syntax: elm parity`. The elm columns of both ratchets reach 100%, shaders aside, with one commit per root cause. Settle the Elm column unit for each Compat from the oracle's regions, and record it in `DECISIONS.md`; 0.19.1 keeps rows and columns in 16 bits, so the Compat reaches validation here.
- [x] **1.17b** `syntax: elm-format parity`. The elm-format columns reach 100%, with one commit per root cause. Two kinds of work:
  - elm-format's legacy syntax: operator definitions such as `(=>) = …`, operators with `$`, `#`, `@`, `~` or Unicode symbols, `\x` escapes and `[a..b]` ranges;
  - its indentation model, which checks only application arguments (`indented`) and block alignment (`checkIndent`), where elm checks every continuation.
- [x] **1.17c** `syntax: GLSL validation`. elm parses each shader with language-glsl and rejects what it can't parse (`Parse/Shader.hs`). Port the subset that decides accept/reject; 1,894 mutations of the shader seed turn on it.
- [x] **1.18** `syntax: fuzz target and benchmarks`.
  - A cargo-fuzz target; tier 1 runs the same property with proptest on stable.
  - `xtask bench syntax`, with a scaling assertion: twice the input must take less than three times as long.

**Exit:**
- Every corpus file parses losslessly with no errors.
- Accept/reject matches elm 0.19.1, 0.19.2 and 0.19.3 and elm-format 0.8.8 on the corpus and the mutations.
- One million fuzz executions run clean.
- 20,000 nested parentheses parse.
- Parsing scales linearly.

## M2: Formatter, release 0.1 (replaces elm-format)

Goal: `nomi format` and the `elm-format` personality, byte-identical to elm-format 0.8.8 and linear in time.

- [x] **2.1** `oracle: formatter snapshots`. Hashes of elm-format 0.8.8's output on the corpus. Import elm-format's own `tests/test-files`, the old fixtures, the bug fixtures and the 119 tuple cases. **Check:** the ratchet exists, at 0 passing.
- [x] **2.2** `format: Box layout engine`. A port of `Box.hs` and `ElmStructure.hs`, starting from the salvaged `document.rs`. **Check:** unit tests mirroring elm-format's behavior.
- [x] **2.3** `format: formatter tree`. Mirrors elm-format's AST (`AST/V0_16.hs` and its neighbors), with C1/C2 comment slots and multiline flags. Types only.
- [x] **2.4** `format: lower module header and imports`. One forward comment cursor, plus a debug assertion that every comment is used exactly once.
- [x] **2.5** `format: lower types and patterns`.
- [x] **2.6** `format: lower expressions`.
- [x] **2.7** `format: lower declarations`.
- [x] **2.8** `format: render header, imports and exposing`. Import normalization; exposing lists grouped by `@docs`. **Check:** the ratchet climbs, as in every render step.
- [x] **2.9** `format: render types and patterns`. Literal spelling came forward with it, since patterns hold literals.
- [x] **2.10** `format: render expressions`. `let` needed definitions and annotations, so `render/declarations.rs` started here.
- [x] **2.11** `format: render declarations and top-level spacing`. Committed after 2.12. Splicing the body verbatim stops here, so whole modules come from the tree. On the corpus, 1505 of 1915 modules pass. Outside doc comments (2.13), every module that differed did so in a literal, and 2.12 settled those.
- [x] **2.12** `format: normalize literals`. Spelling started in 2.9 (`render/literal.rs`). GHC's `Data.Char` predicates (`ghc.rs`) use `icu_properties` pinned at Unicode 15.1, which matches the oracle on every code point probed. Float digits are a full port of GHC's `floatToDigits`, which excludes the rounding boundaries and so is not always shortest (`1.0e23` prints `9.999999999999999e22`). A letter, digit or `_` after a `\x` escape's digits makes upstream's `chompHex` return -2, which is written out as a lone 0xFE byte and read back with the bytes after it (`"\x202Bا…"` → `\u{3D8A1B}\u{008C}…`; abadi199/intl-phone-input's `tests/FilterTest.elm`). Landed before 2.11, because 2.11's corpus ratchet only sees literals once declarations render.
- [x] **2.13** `format: doc comments`. Markdown normalization, and Elm code blocks through the 1.14 entry points. Decided for a Cheapskate port (`markdown/`, see `DECISIONS.md`). Lowering parses the code blocks, and the `@docs` names that order the exposing list now come from the Markdown parse, which replaced 2.8's `render/docs.rs`. Upstream's code-block parsers had more rules than 1.14 knew:
  - `commitIf` backtracks, so the "letter or `_`" start rule never applied and is gone;
  - a block must end with its last entry, except for a `--` comment after a one-line expression or custom-type variant;
  - an expression block can't open with `{- -}`;
  - a `-` starting a later line subtracts;
  - `freshLine` leaves comments after the last line end to the body.

  The corpus passes 1915 of 1915 modules.
- [x] **2.14** `format: check invariants on corpus and proptest`. Four invariants:
  - formatting is idempotent;
  - the output parses;
  - every comment survives;
  - the token stream is unchanged apart from elm-format's normalizations (5.13 upgrades this to "HIR unchanged").

  Done: every one holds wherever elm-format 0.8.8 keeps it. Where upstream breaks one (comments in the header, comments and doc-comment Markdown that move on a second run, `\x` escapes, `{-- }` comments, comments nested in doc comments, `(f x) y` spanning lines, a headless module's type and alias of one name, floats too large for a double), nomi breaks it the same way, oracle-checked. The fixtures' and the corpus's `invariants.tsv` list those modules for 2.15 to decide.
- [x] **2.15** `format: reach corpus parity`. Work the ratchet to 100%, with one commit per root cause. Deviations are allowed only for crashes, non-idempotence, meaning changes or unparseable output, and each gets a fixture and a `DECISIONS.md` entry.
  Done: the corpus formats 1915 of 1915 modules as elm-format 0.8.8 does, and the fixtures 1028 of 1028. nomi deviates in five places where upstream's output of code Elm accepts doesn't parse or reads differently: comments the docs move into a one-line exposing list, `{--` comments, floats too large for a double, doc comments whose reprint ends elsewhere, and negated negations; each has a `format_fixtures/deviations/` case and a `DECISIONS.md` entry. The other invariant breaks stay upstream's, for the reasons `DECISIONS.md` gives.
- [x] **2.16** `format: linear time and depth safety`. The 136, 274 and 549 KB benchmark scales linearly, and 5,000 nested expressions format.
  Done: `cargo xtask bench format` times a module of `Element`'s declarations at 132, 260 and 517 KiB (17, 34 and 69 ms) and five nested and chained shapes; each doubling costs 2.05 to 2.22 times as much. Two quadratics went: a node's range walked to its last token, and joining lines copied the inner one at every level. Recursive lowering and rendering grow the stack where it runs low, so `tests/depth.rs` formats twelve shapes nested 5,000 deep.
- [x] **2.17** `engine: VFS and thread pool`. Reads, atomic writes, file discovery, and the process's only pool. This is pulled forward from M4 so the `format` crate never touches files.
  Done: `nomi-engine` has a `Vfs` that reads UTF-8, replaces a file by renaming a sibling over it (keeping its permissions and any symbolic link), and discovers modules as elm-format 0.8.8 does: every `.elm` file under a directory, following links, skipping `elm-stuff`, `node_modules` and `.git`, hidden directories included; an explicit file is itself. Where elm-format lists a directory in the disk's order, nomi sorts by name. `Pool::global` is a rayon pool whose threads get the core stack, and `cargo xtask ci` fails if any other crate depends on a pool crate.
- [x] **2.18** `cli: nomi format`. Paths, `--check` and `--stdin`, run in parallel through the engine. `fmt` is an alias.
  Done: `nomi_engine::format` discovers the modules under each path, formats each once on the pool and rewrites only those that changed; `--check` lists the unformatted ones instead. Exit 1 on an unformatted module under `--check`, a module elm-format rejects (reported at its first error's row and column), an I/O error, or a path without `.elm` files. The CLI maps the flags and prints the report. `nomi_format::format_module` returns the first error where `format` returned `None`.
- [x] **2.19** `cli: elm-format personality`. Under the name `elm-format`: flags, prompts, messages, `--validate` JSON and exit codes. **Check:** transcript snapshots against elm-format 0.8.8.
  Done: invoked under a name whose stem is `elm-format`, the CLI parses arguments as elm-format's optparse-applicative parser does (left to right, the first help or error wins, INPUTs collected last first) and prints its help byte for byte; `nomi_engine::elm_format` ports `Cli.hs` and `TransformFiles.hs` behind a `Console`, formatting on the pool. `cargo xtask oracle transcripts` records elm-format's transcript of each case under `crates/cli/tests/elm_format/cases/`, and all 57 match; three `deviations/` cover Elm 0.17 and 0.18. The remaining differences, in `DECISIONS.md`, are those where elm-format depends on disk or thread order, invalid UTF-8, I/O failures, and syntax error positions, which 4.6a takes on.
- [x] **2.20** `release: 0.1`. README usage (`ln -s nomi elm-format`), benchmarks, and tag `v0.1.0`.
  Done: `cargo xtask bench elm-format` times nomi as `elm-format` against 0.8.8 on the four corpus apps, validating each whole (59 to 583 modules) and piping its largest module through `--stdin`; nomi takes 4 to 86 ms, five to twelve times less, and both exit alike. The workspace version is 0.1.0; the tag is local.

**Exit:** byte parity on the corpus, the invariants hold on the corpus and under fuzzing, time is linear, and the elm-format CLI transcripts are identical. You review the deviation list and try it in your editor; editors call `elm-format --stdin`.

## M3: Projects and packages

Goal: read and write `elm.json` exactly as elm does, resolve and fetch dependencies, and provide `init` and `install`.

- [x] **3.1** `project: elm's JSON`. Upstream's own JSON parser and decoder (`Json/Decode.hs`), not serde: its parse errors and positions per Compat (0.19.1 counts rows and columns in 16 bits and reads strings without checking UTF-8), its tree of decode problems, with strings kept raw and escapes undecoded, and the layout of `Json/Encode.hs`'s `write`. **Check:** unit tests for every parse error, and a fuzz target that encodes what it decodes back to the same tree.
- [x] **3.2** `project: names, versions and constraints`, with upstream's validation rules, positions and per-Compat differences (0.19.3 counts module-name columns in bytes and packs versions differently), plus the SPDX license table and its suggestions. The operations the solver and `add` need (`satisfies`, `intersect`, `untilNextMajor` and the like) come with them.
- [x] **3.2a** `project: elm.json model`. Application and package outlines, with parse errors classified as upstream does them, and re-serialization byte-identical to elm's. **Check:** every corpus `elm.json` round-trips; error fixtures, including the `elm.json` cases from the old CLI report matrix (see 7.5), whose classification and position match reports recorded from each elm.
- [x] **3.3** `project: Compat and paths`. `Compat` comes from `elm-version`. Root discovery works as upstream's does. A single module computes every nomi path.
- [x] **3.4** `registry: package index`. `/all-packages` plus incremental `/all-packages/since/N`, usable offline. HTTP sits behind a trait, and tests use a fixture registry.
- [x] **3.5** `registry: package store`. Downloads `endpoint.json` and the zip, verifies the hash, extracts with hardening, downloads in parallel, and stores by content.
- [x] **3.6** `oracle: solver snapshots`. Rebuild the old Haskell solver oracle (`conformance/oracles/package-solver/Main.hs`) over a frozen index; this may need GHC 9.8 (D3). Scenarios come from the old solver tests plus generated ones, including cases with several valid answers and conflicts that force backtracking, and `install`'s relaxation order. It comes before the solver so its selection policy is tested from the start. **Check:** the ratchet exists, at 0 passing.
- [x] **3.7** `registry: pubgrub solver`. Must select what upstream selects, not just a valid answer. Upstream's `Deps/Solver.hs` takes pending packages in name order (`Map.minViewWithKey`), tries versions newest first, and keeps the first success of a depth-first search; for `install`, the relaxation order in `Install.hs`. Failures come with an explanation. If PubGrub's priorities can't reproduce that selection on the 3.6 cases, select with a port of upstream's search and keep PubGrub for explanations, with a `DECISIONS.md` entry.
- [x] **3.8** `registry: reach solver parity`. Work the ratchet to 100%.
- [x] **3.9a** `base: terminal renderer`. A port of ansi-wl-pprint's `renderPretty 1 80`, plain and with ANSI styles, since `init` and `install` print documents before 4.4. 4.4 adds snippets and the rest of `Report.hs`. **Check:** unit tests of the layout rules, and the transcripts of 3.9b and 3.9.
- [x] **3.9b** `oracle: init and install transcripts`. Real elm in an offline sandbox whose registry and package cache hold corpus packages, so `install` can build them. **Check:** transcripts recorded from each elm.
- [x] **3.9** `cli: new and add`. `nomi new` (alias `init`) and `nomi add` (alias `install`), plus the `elm init` and `elm install` personalities: prompts, the printed plan, and `elm.json` edits. **Check:** transcript snapshots recorded from elm.
- [x] **3.10** `cli: rm`. Removes a direct dependency and keeps the versions of what remains (DECISIONS 2026-10-08), so indirect dependencies nothing else needs disappear too. Test dependencies are handled the same way. Alias `remove`. **Check:** fixture-registry scenarios; the result is what `elm install` would produce for the remaining direct dependencies.
- [x] **3.11** `cli: update, outdated and tree`. `update` re-solves for the newest versions of the same major version (DECISIONS 2026-10-08), optionally for one package and the packages that depend on it, and prints the plan before writing. `outdated` and `tree` only read. **Check:** fixture-registry scenarios.

**Exit:** solver parity, byte-identical `elm.json` rewrites, and every corpus app's dependencies resolve and fetch.
  Done: `cargo xtask corpus deps` checks each corpus app's dependencies out and downloads them, as every Compat, through `nomi_registry` against the live registry: 456 releases. It checks archives against the registry's hashes, not pins of ours, so it runs by hand like `corpus pin`, not in CI.

## M4: Engine, reports and syntax errors (with LSP v0)

Goal: `nomi check` reports syntax and module errors exactly as `elm make` does, and the language server runs on the same engine.

- [x] **4.1** `engine: revisions, overlays and FileIds`. A versioned VFS with editor overlays, and FileIds canonical across symlinks (R7).
  Done: `Vfs` lays editor overlays over the disk, and each overlay set or removed, write, or reported disk change makes a new `Revision`. A `Snapshot` keeps its revision's overlays, reads the disk when asked, and knows when it is stale (DECISIONS 2026-10-08). `file_id` resolves symbolic links, through the nearest existing directory for unsaved files, and interns the path.
- [x] **4.2** `engine: module graph`. Crawls the way upstream's `Build.hs` does: module ↔ path, packages' exposed modules, and the not-found, ambiguous, name-mismatch and cycle errors.
  Done: `graph::Graph::{from_paths, from_exposed}` crawl in parallel waves, check roots as `findRoots`/`checkRoots` do, and report each release's cycle (DECISIONS 2026-10-08), checked against all three elm releases on random graphs.
- [x] **4.3** `engine: scheduler and cache`. The module DAG runs on the pool, with cancellation between phases, a self-trimming content-addressed per-user cache, and keys built from interface hashes.
  Done: `schedule::run` starts each module when its last import finishes, blocks the importers of failures, and stops starting modules once the snapshot is stale. `artifacts::Artifacts` files payloads under `Key` hashes in the user's cache, with the salvaged envelope and atomic writes, and trims like `GOCACHE` (DECISIONS 2026-10-08). A toy build checks early cutoff.
- [x] **4.4** `base: report documents and renderers`. A port of `Reporting/Doc.hs`, `Render/Code.hs` and `Report.hs`, with the terminal renderer of 3.9a extended to snippets, a `--report=json` renderer, and regions that depend on Compat.
  Done: documents name snippets and pairs by range; `code::expand` puts in each release's lines, numbers and carets, `render::chunks` cuts a layout where styles change as `D.encode` does, and `json` encodes messages, regions and reports byte for byte (DECISIONS 2026-10-08). Goldens from all three releases cover pairs, regions over rows and tabs.
- [x] **4.4a** `report: elm.json and dependency problems`. `toOutlineReport` and `toDetailsReport`, which `install`, `make` and the rest share, with their snippets. Until then 3.9's commands print a plain one-line version. **Check:** transcripts from elm of broken `elm.json` files and hand-edited dependencies.
  Done: `nomi_project::report` reports parse and decode errors and the outline checks, and `nomi_registry::report` details and package problems, for `install` and every command that loads a project. 41 fixtures' reports from all three releases match byte for byte, and transcripts cover missing and redundant source directories (DECISIONS 2026-10-08).
- [x] **4.5** `oracle: syntax error report snapshots`. Full JSON reports for the mutation suite, for each Compat.
  Done: `cargo xtask oracle syntax` keeps a 64-bit SHA-256 prefix of each release's `--report=json` for every mutation it rejects, in `syntax_mutations/reports/<seed>.tsv`, since the reports themselves, each quoting its mutation, would take over a hundred megabytes. `--show '<seed> / <mutation>'` runs one mutation again and prints the reports.
- [x] **4.6** `syntax: elm's parser, for its errors`. A port of upstream's `Parse/*.hs` and the error types of `Reporting/Error/Syntax.hs`, for every Compat, run on a module the grammar rejects. It gives upstream's error constructor, contexts and cursors, from which 4.7 to 4.10 build the title, region and message (DECISIONS 2026-10-08). **Check:** on the mutation suite it rejects exactly what each elm rejects.
  Done: `nomi_syntax::elm::parse` ports `Parse/*.hs` with upstream's error types, one parser for every Compat. `tests/elm_parser.rs` holds it to every cell of `syntax_mutations/oracle/`: across the three releases it accepts the 29,409 cells elm accepted and rejects the other 88,533. The shader error's Parsec message is left empty until 4.10.
- [x] **4.6a** `syntax: elm-format's error positions`. elm-format reports a syntax error where its own parser stops: often a column past the start of the token elm reports (4,447 `Unexpected` and 800 `ExpectedDeclaration` cases), and with tabs and carriage returns counted its own way. nomi's first error is at the same position on 17,898 of the mutation suite's 27,814 rejected cases. So `nomi_syntax::elm_format` ports elm-format's `Parse/*.hs` for 0.19, as 4.6 ports elm's, and `format_text` reports its position when nomi's grammar rejects a module (DECISIONS 2026-10-08). **Check:** on the mutation suite it rejects exactly what elm-format rejects, at the position elm-format reports, on all 27,814 cases; held exact, not ratcheted.
- [x] **4.7** `report: syntax errors in headers and imports`. The first part of `Reporting/Error/Syntax.hs` (5,904 lines), and two ratchets over the mutation suite: one on titles and regions, against `syntax_mutations/oracle/`, and one on full reports, against `syntax_mutations/reports/`. **Check:** both ratchets climb, as in 4.8 to 4.10.
- [x] **4.8** `report: syntax errors in declarations and types`.
- [x] **4.9** `report: syntax errors in expressions and patterns`.
- [x] **4.10** `report: syntax errors in literals, strings, numbers and shaders`.
  Done: every syntax error has its report, so `elm::to_problem` returns one outright. Titles and regions match each elm on all 29,511 rejected cases of the mutation suite, and full reports on 27,616. The other 1,895 are SHADER PROBLEM reports, whose message quotes language-glsl's Parsec error, which 4.10a ports.
- [x] **4.10a** `syntax: language-glsl's error messages`. The GLSL recognizer tracks only where Parsec fails, but elm quotes Parsec's whole message (`showErrorMessages`), such as `unexpected "v"` and `expecting "/", space, "(", "[", "=", "," or ";"`. So each rule carries Parsec's labels and messages too, and `ShaderProblem` gets the message. **Check:** the report ratchet passes every case of the mutation suite.
  Done: `glsl/error.rs` ports Parsec's `ParseError`, its merging, `<?>` and `showErrorMessages`, with GHC's `show` for what it quotes; the recognizer's primitives carry Parsec's labels (`char`'s `show`, `space`, `letter`, `eof`'s "end of input", `buildExpressionParser`'s `""` and "operator"). elm 0.19.1 links Parsec 3.1.13.0, 0.19.2 3.1.17.0 and 0.19.3 3.1.18.0, all pinned in `references.toml`; nothing a shader's error shows differs between them. Full reports match each elm on all 29,511 rejected cases.
- [x] **4.11** `report: module and import errors`, the matching parts of `Reporting/Exit.hs`.
  Done: `nomi_engine::report` ports `Reporting/Error/Import.hs` with `toImportErrors`, the MODULE NAME MISSING and MISMATCH reports of `Syntax.hs`, and `toProjectProblemReport` for what the crawl finds, `BP_MissingExposed` included. `cargo xtask oracle modules` records each elm's `--report=json` on 22 projects in `crates/engine/tests/modules/cases/`, run in a directory padded to 96 bytes so messages that name it wrap alike, and `tests/module_reports.rs` matches them. `Env::with_cwd` resolves root paths as elm's working directory does, so reports name them as given. elm's arguments take only `.elm` files, so UNEXPECTED FILE EXTENSION, like AMBIGUOUS IMPORT between packages, has unit tests instead.

**Review fixes (2026-10-08).** An outside review at `ab21774` found nine defects in code already written (CR.10 turned up while fixing them). They are fixed here, before 4.12 builds on them, in this order. The first four were reproduced; the rest are plain from the code.

- [x] **CR.8** `registry: reject overflowing cached index lengths`. `Index::decode` ([registry/src/index.rs](../crates/registry/src/index.rs)) adds a cached length to its offset unchecked; the magic header followed by a decimal `usize::MAX` panics in debug builds. **Check:** that input returns `None`.
- [x] **CR.2** `engine: a run that ends stale is cancelled`. `schedule::run` checks the snapshot only before each module starts, so a last job that makes it stale still returns `Ok`. Check it once more when the run ends; publishing editor results checks it again (4.13). **Check:** invalidating the snapshot during the last job returns `Cancelled`.
- [x] **CR.3** `engine: resume resolving links after a missing directory`. `vfs::canonical` stays lexical after a missing component even when `..` climbs back to an existing directory, so with `link` naming `real`, the unsaved `missing/../link/New.elm` and `real/New.elm` get different `FileId`s (R7). **Check:** both have one id, and an overlay set through one reads and removes through the other.
- [x] **CR.7** `test: make tier 1 hold on Windows`. Two symlink tests in [engine/tests/vfs.rs](../crates/engine/tests/vfs.rs) use `std::os::unix` unconditionally; `module_reports.rs` compares against reports recorded on Linux, while the cycle report draws in ASCII on Windows and paths use `\`; and `ci.yml` runs only on pushes to `master`, not `rewrite`. **Check:** the test targets compile for Windows, and CI runs on `rewrite`.
  Done: the two tests are Unix-only, as `module_reports.rs` is, and `ci.yml` runs on pushes to `rewrite`. No Windows target is installed here, so the compile check waits for the first Windows CI run, which has never happened; expect more there.
- [x] **CR.4** `runtime: include fibers spawned during shutdown`. `Runtime.shutdown` ([runtime/src/fiber.js](../runtime/src/fiber.js)) waits only for the fibers present when it starts, so a daemon a finalizer forks outlives it. **Check:** a finalizer that forks a daemon leaves nothing alive when shutdown resolves; finalizers still run exactly once and both fiber tables end empty.
- [x] **CR.1** `report: a module nomi rejects always has a report`. `report::module_problem` takes elm's diagnostic parser's error with `.err()?`, so a module only nomi's grammar rejects, such as 25,001 nested parentheses past `DEPTH_LIMIT`, fails with no report. Report nomi's own error there; elm has no depth limit, so the limit gets a `DECISIONS.md` entry. **Check:** that module produces a report.
- [x] **CR.6** `engine: key artifacts by the build that made them`. `artifacts::Key::new` names the toolchain by `CARGO_PKG_VERSION`, which every development build shares. Key by a hash of the running binary instead, as Go's build ID does. **Check:** a different build misses where the same build hits.
- [x] **CR.10** `syntax: drop deep trees on a stack that holds them`. Found during CR.5: rowan drops a tree recursively, and the graph keeps each module's `Parse`, so a graph built on the pool but dropped on a caller's thread overflows its stack: a module nested 24,999 deep, which elm and nomi accept, aborts on an 8 MiB thread, and 3,000 deep on a 1 MiB one, as Windows' main thread is. **Check:** a graph of a module nested to the limit, built on the pool, drops on a 1 MiB thread.
- [x] **CR.5** `engine: keep the recovered syntax of a broken module`. `graph::read` drops the tolerant parse of a module with syntax errors. Keep it in `BadModule` for editor consumers (R3, R4); the crawl still stops there, as `Build.hs` does, which decides what `elm make` reports. Crawling on through broken modules is for M5's navigation. **Check:** a module with a broken declaration after a valid import keeps its tree and imports, while the graph's reports are unchanged.
- [x] **CR.9** `test: resolve the corpus digest mismatch and rerun tier 2`. `cargo xtask corpus syntax` finds 30,463 modules where `corpus/syntax/oracle.tsv` was made from 30,448. Find the difference against the pins before deciding whether to regenerate the oracle, which is your call. **Check:** `cargo xtask corpus syntax` and `cargo xtask corpus format` reach their parity checks and pass.
  Done: the count was 30,448 all along; one path differed. jxxcarlson/meenylatex 14.1.1's zipball has `Demo/src/*.elm` and `demo/src/Main.elm`, which the case-insensitive filesystem the oracles ran on unpacked as one `Demo`, and a case-sensitive one keeps apart. `corpus fetch` now unpacks names that differ only in case as the archive's first spelling, and unpacks again a package that has both. Both suites pass 1915/1915 against the oracles as they were.

- [x] **4.12** `cli: nomi check with elm make's reporting policy`. Only the earliest failing phase per module is shown, and blocked modules stay silent.
  Done: `nomi check [paths] [--report json]` is `elm make --output=/dev/null` as far as nomi's phases go. `nomi_engine::check` loads the project and its dependencies, downloading what the store lacks as `Details.load` does, crawls the paths or a package's exposed modules, and reports the project's problem or each failing module's: outside roots first, as given, then modules by name, each release ordering names its own way (`gatherProblemsOrMains`); the terminal lists them oldest file first, with upstream's separators (`Error.toDoc`), and says on standard output how many modules failed. With no later phases yet, a module whose imports failed has nothing to show, and the crawl already stops at a broken module. `cargo xtask oracle modules` now also records elm's terminal report, with each case's files dated a second apart, and `tests/module_reports.rs` holds `nomi check` to both reports, whole, on 25 projects from each release. Where nothing nomi checks fails, it says so and that names and types aren't checked yet.
- [x] **4.13** `lsp: v0`. New `ide` and `lsp` crates.
  - The `lsp-server` loop and position encodings.
  - Overlays and syntax diagnostics, published only while their snapshot is current (CR.2).
  - Formatting as a minimal diff.
  - Document symbols.

  **Check:** transcript tests on the salvaged harness.

  Done: `nomi lsp` serves the protocol on standard streams through `lsp-server`. `nomi_ide` answers over engine snapshots in byte ranges: elm's syntax report for a file, read as its nearest `elm.json`'s release and kind of source; formatting as one edit per run of changed lines, trimmed to the characters that change; and document symbols as elm-language-server 2.10.0 lists them, flat, `let` values included. `nomi_lsp` negotiates UTF-8 or UTF-16 columns, applies incremental changes to the open documents, lays them over the disk as overlays, and works out every open document's diagnostics on the pool after each change, publishing under the lock that makes revisions only while the snapshot is current, so an edit to one file never leaves another's unpublished or stale ones last. `tests/transcript.rs` drives the server on an in-memory connection, as the first attempt's harness did, through the handshake, diagnostics that come and go, closing, formatting, symbols and an unanswered request; a CLI test speaks to `nomi lsp` over its streams.

**Exit:** syntax error titles and regions are identical on the mutation suite for every Compat, and the message-text ratchet is tracked. `nomi lsp` works in your editor.

## M5: Names (canonicalization)

Goal: every module elm canonicalizes, nomi canonicalizes to the same names, and every naming error elm reports, nomi reports with the same title, region and text.

Upstream canonicalizes a parsed `Src.Module` in one order (`Canonicalize/Module.hs`): the import environment (`Environment/Foreign.hs`), the module's own types, constructors and values (`Environment/Local.hs`, `Dups.hs`), each value (`Expression.hs`, `Pattern.hs`, `Type.hs`), cycle detection over the values, effects, and exports. It collects every error it meets (`Reporting.Result`), so which errors a module shows, and in what order, follow that order; nomi's steps keep it. The HIR is nomi's `Src.Module` with IDs: lowering keeps what the source wrote, and canonicalization writes what each name means into side tables keyed by ID, plus the operator trees and declaration order it decides. A package's exposed values must be annotated (`Elm/Docs.hs`), so a package's interface is complete without type inference; an application module's unannotated values get their types in M6.

- [ ] **5.1** `compiler: create the crate, the HIR and its source map`. The pure `nomi-compiler` crate. Arenas with IDs (`ExprId`, `PatId`, `TypeId`, `DefId`) for a module's header, imports, declarations, expressions, patterns and types as written, with operator chains still flat, and a source map from each ID to its CST node. **Check:** every corpus module elm accepts lowers, every ID maps to a node whose text is the construct's, and a 20,000-deep expression lowers and drops.
- [ ] **5.2** `compiler: interfaces`. `Elm/Interface.hs`: exposed values with their canonical types, unions (open, closed, private), aliases and binops, as the public and private forms upstream keeps for dependencies; with a canonical type representation (`AST/Canonical.hs`'s `Type`, `Union`, `Alias`), stable serialization, and a hash. **Check:** serialization round-trips and the hash is stable under rebuilding the same interface.
- [ ] **5.3** `test: hermetic package fixtures`. Pinned copies of elm/core, json, html, browser, url, virtual-dom, time, http, bytes and file under `crates/compiler/tests/packages/`, with their licenses in `THIRD-PARTY-NOTICES.md`, so tier 1 canonicalizes real code offline. **Check:** each copy matches its registry hash.
- [ ] **5.4** `oracle: naming error snapshots`. `cargo xtask oracle names` records each elm's `--report=json` and terminal report, as `oracle modules` does, on curated projects (one or more per constructor of `Reporting/Error/Canonicalize.hs`'s `Error`) and on mutations of a few real modules over the fixture packages: renamed and removed names, names taken out of exposing lists, duplicates, shadowing, bad cycles. A tier-1 test holds nomi's titles and regions to them (a ratchet, climbing in 5.5 to 5.13), and a second one the full reports (climbing in 5.14). **Check:** the snapshots are checked in and the ratchets exist.
- [ ] **5.5** `compiler: import environment`. `Environment/Foreign.hs`: default imports, qualified, unqualified and aliased imports, exposing lists including `(..)` and constructors, kernel imports for `elm/*` and `elm-explorations/*`, and which names are ambiguous, with the errors upstream raises there.
- [ ] **5.6** `compiler: local declarations and duplicates`. `Environment/Local.hs` and `Dups.hs`: the module's types, aliases, constructors and values added over the imports, with duplicate and shadowing errors.
- [ ] **5.7** `compiler: canonicalize types and aliases`. `Type.hs` and the alias checks in `Local.hs`: type variables, arity, recursive and badly recursive aliases.
- [ ] **5.8** `compiler: canonicalize patterns`. `Pattern.hs`, with duplicate bindings.
- [ ] **5.9** `compiler: canonicalize expressions`. `Expression.hs` but operators and recursion: variables of every kind, records, updates, accessors, `let`, `case`, lambdas, shaders, and the shadowing rules of each.
- [ ] **5.10** `compiler: operator precedence`. Flat chains become trees by the fixities in scope, as `Expression.hs`'s binop pass does, with its errors.
- [ ] **5.11** `compiler: declaration order and recursion checks`. `Module.hs`'s two cycle passes over top-level values and `Expression.hs`'s over `let`, ordered as `Data.Graph.stronglyConnComp` orders them (reusing the port from 4.2).
- [ ] **5.12** `compiler: ports, effect managers and exports`. `Effects.hs` and `canonicalizeExports`, then the module's interface (5.2).
- [ ] **5.13** `engine: canonicalize the build`. `nomi check` canonicalizes each module after its imports, with package interfaces from the dependencies' sources, cached as artifacts keyed by their interfaces; a module whose imports failed stays silent. `cargo xtask corpus names` is the tier-2 suite: every corpus package and app each Compat accepts canonicalizes cleanly, with each package's dependencies fetched as `corpus deps` fetches them.
- [ ] **5.14** `report: naming errors`. A port of `Reporting/Error/Canonicalize.hs` (1,256 lines) and `Reporting/Suggest.hs`, split here if it outgrows a step. **Check:** the full-report ratchet of 5.4 reaches every case.
- [ ] **5.15** `format: check the HIR-unchanged invariant`. Formatting a module leaves its HIR unchanged, up to IDs and source positions; replaces the token-level check from 2.14.
- [ ] **5.16** `ide: definition, references and highlights`, from the side tables, on the module and across the project.

**Exit:** every corpus package and app each Compat accepts (1.15a) canonicalizes cleanly, naming errors match on the 5.4 snapshots in title, region and text for every Compat, and navigation works in the editor.

## M6: Types

- [ ] **6.1** `compiler: type variables and union-find`. Descriptors, ranks and marks (`Type/Type.hs`, `UnionFind.hs`).
- [ ] **6.2** `compiler: unification`. Covers the super types (number, comparable, appendable, compappend) and records, and keeps aliases for messages.
- [ ] **6.3** `compiler: constraints for expressions`, in upstream's order.
- [ ] **6.4** `compiler: constraints for patterns`.
- [ ] **6.5** `compiler: annotations, rigid variables and module constraints`.
- [ ] **6.6** `compiler: solver`. Pools, generalization, the occurs check and instantiation.
- [ ] **6.7** `compiler: typed interfaces for packages`. **Check:** every corpus package type-checks under each Compat that accepts it (1.15a).
- [ ] **6.8** `compiler: per-expression types`. A side table, added without changing the solving order.
- [ ] **6.9** `oracle: type error snapshots`. Curated cases plus mutations (swapped literals, swapped arguments, edited annotations), for each Compat.
- [ ] **6.10** `compiler: type error data`. `Type/Error.hs`: categories, expectations, diffs and problems.
- [ ] **6.11** `report: type errors in annotations and mismatches`. The first part of `Reporting/Error/Type.hs` (1,619 lines). **Check:** the ratchet climbs, as in 6.12 to 6.14.
- [ ] **6.12** `report: type errors in calls and operators`.
- [ ] **6.13** `report: type errors in branches and patterns`.
- [ ] **6.14** `report: records, infinite types and hints`.
- [ ] **6.15** `engine: early cutoff`. **Check:** an engine test that counts phase runs shows that a body-only edit re-checks one module.
- [ ] **6.16** `ide: hover types and completion v0`.

**Exit:** accept/reject matches across the corpus, type errors match, and a warm rebuild after a body edit re-checks only that module.

## M7: Remaining checks and docs

- [ ] **7.1** `compiler: exhaustiveness and redundancy`. A port of `Nitpick/PatternMatches.hs` and its reports.
- [ ] **7.2** `compiler: Debug check for --optimize`.
- [ ] **7.3** `compiler: main validation`. `addDefHelp` and `checkPayload`, with their reports.
- [ ] **7.4** `compiler: package docs`. `Elm/Docs.hs` and the docs errors. **Check:** `docs.json` is byte-identical for every corpus package each Compat accepts, and the rejected ones fail with upstream's error.
- [ ] **7.5** `oracle: whole-project snapshots`. `elm make --output=/dev/null --report=json` on every corpus project, for each Compat, in the 1.15a sandboxes; rejected projects are expected-failure cases. Add the old 605-case CLI report matrix (`conformance/checkpoints/2026-09-23/json-report-fixtures`), which compares every output stream, exit status and artifact.
- [ ] **7.6** `cli: reach nomi check parity`. Work the ratchet to 100%.

**Exit:** `nomi check` matches `elm make --output=/dev/null` across the corpus and every error corpus.

## M8: Codegen, release 0.2 (replaces elm)

- [ ] **8.1** `compiler: optimized graph`. `AST/Optimized.hs` and `Optimize/Expression.hs`: tail calls and generated names.
- [ ] **8.2** `compiler: decision trees`. `Optimize/DecisionTree.hs` and `Case.hs`.
- [ ] **8.3** `compiler: ports and effect managers` (`Optimize/Port.hs`).
- [ ] **8.4** `codegen-js: kernel chunk parser` (`Elm/Kernel.hs`).
- [ ] **8.5** `codegen-js: builder and names`. Exact whitespace, plus the salvaged string escaping, `ElmChar` and BigInt literals.
- [ ] **8.6** `codegen-js: expressions`. Currying helpers, records, decision trees and tail loops.
- [ ] **8.7** `codegen-js: program graph`. Reachability from each `main`, ordering, cycles and `_Platform_export`.
- [ ] **8.8** `codegen-js: dev, debug and optimize modes`. Field shortening, and debugger metadata (`Elm/Compiler/Type/Extract.hs`).
- [ ] **8.9** `codegen-js: html and /dev/null outputs`.
- [ ] **8.10** `oracle: JS output snapshots`. The four apps in three modes for each Compat (with 1.15a's `elm-version` rewrite), plus fixture programs. The old manifest already has the 0.19.2 hashes.
- [ ] **8.11** `codegen-js: reach byte parity`. Work the ratchet to 100%.
- [ ] **8.12** `engine: object cache and linking`. Optimized graphs are cached per module, and linking takes only what `main` reaches.
- [ ] **8.13** `cli: nomi build and elm make`. `nomi build` (alias `make`, with `--release` as an alias of `--optimize`) and the `elm make` personality. Flags (including `--docs`), progress and success messages, and exit codes. **Check:** transcripts, including the terminal output of the 605-case matrix.
- [ ] **8.14** `cli: bump, diff and publish`. The API diff is computed from docs. Publishing is tested only against the fixture registry; publishing for real is yours to run.
- [ ] **8.15** `cli: explain missing repl and reactor`. The message for `elm reactor` points to `nomi run --watch`.
- [ ] **8.16** `release: 0.2`. README usage (`ln -s nomi elm`), cold and warm build benchmarks against elm, and a tag.

**Exit:**
- Generated JS is byte-identical on the app corpus in every mode and Compat.
- `elm` transcripts are identical.
- The syntax, naming and type error text ratchets are at 100%, apart from listed deviations.
- A cold build takes at most 1.5× elm's time, and warm builds are faster.

## M9: Language server, release 0.3 (replaces elm-language-server)

The parity list comes from elm-language-server 2.10.0:

- **Diagnostics:** compiler errors, its own lint rules (unused imports, values, patterns and the rest), and elm-review run through its CLI.
- **Navigation and editing:** hover, definition, type definition, references, rename, completion with auto-import, document and workspace symbols, folding, selection ranges, linked editing, code lens and formatting.
- **Code actions:** its 15 code actions, plus elm-review fixes.
- **Custom requests:** `elm/findTests`, expose/unexpose, move, and virtual file contents.

Semantic tokens, inlay hints and signature help aren't in elm-language-server; they come after parity.

- [ ] **9.1** `lsp: transcript oracle`. The salvaged harness, with sessions recorded from elm-language-server 2.10.0 on corpus projects.
- [ ] **9.2** `ide: diagnostics policy and debouncing`.
- [ ] **9.3** `ide: lint diagnostics`, porting elm-language-server's rules.
- [ ] **9.4** `ide: completion`. Names in scope, qualified names, record fields resolved by type, and auto-import.
- [ ] **9.5** `ide: rename`, including exposing lists and prepare-rename.
- [ ] **9.6** `ide: workspace symbols, folding, selection ranges and linked editing`.
- [ ] **9.7** `ide: code lens`.
- [ ] **9.8** `ide: code actions for imports, annotations and exposing`.
- [ ] **9.9** `ide: code actions for case branches, record fields, union variants and declarations from usage`.
- [ ] **9.10** `ide: code actions for extracting, moving, adding parameters and removing unused code`.
- [ ] **9.11** `lsp: custom requests and elm-review`.
- [ ] **9.12** `lsp: performance budgets`. On the largest app: time to first diagnostics, keystroke to diagnostics, and memory.
- [ ] **9.13** `release: 0.3`. Editor setup for VS Code, Neovim, Helix and Zed.

**Exit:** transcript parity, budgets met, and you've used it on a real project.

## M10: Test runner, release 0.4

- [ ] **10.1** `oracle: elm-test transcripts`. elm-test 0.19.3-0 on sample suites with fixed seeds, using the console, JSON and JUnit reporters.
- [ ] **10.2** `test: discovery by type`. Every exposed value of type `Test`.
- [ ] **10.3** `test: runner generation`. A generated `main` over elm-explorations/test's runner API, built through the engine.
- [ ] **10.4** `test: Node workers`. `--seed`, `--fuzz`, filters and the worker count.
- [ ] **10.5** `test: reporters`. Console, JSON and JUnit, byte-identical.
- [ ] **10.6** `cli: nomi test and the elm-test personality`.
- [ ] **10.7** `release: 0.4`.

**Exit:** output matches elm-test for fixed seeds, and the corpus apps' test suites pass.

## M11: Node platform and server runtime, release 0.5

This needs M8 and lane R. At the start of M11 we review package names and APIs together, after writing real server code against drafts.

- [ ] **11.1** `codegen-js: ES module output and entry shim`, with optional source maps.
- [ ] **11.2** `codegen-js: platform availability check`. Runs after DCE and names the import path in the error (R1).
- [ ] **11.3** `runtime: Node kernels`. `Scheduler` on the lane-R runtime, the `Platform` lifecycle, and `Http` via `fetch`. Upstream's `Process.sleep` calls `setTimeout` and the `Time` kernel calls `Date.now` and `setInterval` directly, so the Node platform also overrides `Process` and `Time` to go through the runtime's clock; the virtual clock then controls `Process.sleep`, `Time.now` and `Time.every`. **Check:** corpus `Platform.worker` programs run unchanged, and a compiled Elm fixture using those three effects runs under the virtual clock with no real waiting.
- [ ] **11.4** `packages: nomi/core`. `Nomi.Program`, `Nomi.program` and `Env`.
- [ ] **11.5** `packages: concurrency`. `Fiber`, `Task.both`, `race`, `timeout`, `acquireRelease` and `uninterruptible`, plus `Ref` and `Queue`.
- [ ] **11.6** `packages: server`. Listen, requests and responses, a fiber per request, cancellation on disconnect, and draining. The lane-R lifecycle interrupts the root at once and scope closure interrupts children first, which would cut in-flight requests off; so draining needs its own phase: on the first signal the server stops accepting, in-flight request fibers keep running until they finish or the deadline passes, then the rest are interrupted and finalizers run. Request fibers belong to the server's scope, not the handler's caller. **Check:** tests for each of the three phases.
- [ ] **11.7** `packages: fs, process and stdio capabilities`. They can be narrowed (`Fs.subdir`) and faked.
- [ ] **11.7a** `project: minimal nomi.toml`. Entries, platform overrides and `ffi` modules, which 11.8 and 11.9 need, in the `project` crate with a strict schema and span errors. 12.1 extends this schema; it doesn't replace it.
- [ ] **11.8** `codegen-js: FFI`. `Ffi.task`, the `ffi` entry in `nomi.toml`, a link-time check of function names, and `AbortSignal`.
- [ ] **11.9** `cli: nomi run`. Builds and runs a Node entry; `--watch` restarts it on change.
- [ ] **11.10** `packages: effectful tests in nomi/test`. Fake capabilities and the virtual clock, which must also drive elm/core's `Process.sleep` and elm/time (11.3). **Check:** the 11.3 Elm fixture, rerun through `nomi/test`.
- [ ] **11.11** `example: API server`. Lifecycle tests:
  - SIGTERM lets an in-flight response complete, then cleans up and exits within the deadline;
  - a client disconnect cancels its request;
  - a defect gives a 500 and a log line;
  - nothing leaks under load.
- [ ] **11.12** `runtime: Node feature probe`. A startup probe instead of version gates.
- [ ] **11.13** `release: 0.5`.

**Exit:** the example server passes its lifecycle tests on the oldest maintained Node LTS, and availability errors show paths.

## M12: Workspaces, release 1.0

- [ ] **12.1** `project: nomi.toml workspaces`. Adds workspace members and server settings to the 11.7a schema.
- [ ] **12.2** `project: workspace graph`. Members, with member packages resolved by name.
- [ ] **12.3** `registry: one resolution per workspace`.
- [ ] **12.4** `engine: multi-project builds`. Identical inputs share artifacts.
- [ ] **12.5** `ide: one owner per file` (R7).
- [ ] **12.6** `cli: workspace-aware commands`. `-p <member>`, or all entries.
- [ ] **12.7** `cli: nomi run for the whole workspace`. Without `-p`, `nomi run` starts every runnable entry in parallel: Node entries as child processes, browser entries through a small static dev server. With `--watch`, one engine rebuilds what changed, then restarts the affected Node entries and reloads the affected pages. Output is prefixed by member, and Ctrl-C drains everything. This replaces `elm reactor`. **Check:** the 12.8 example, driven by a test that edits a shared module.
- [ ] **12.8** `example: full-stack workspace`. `apps/web` runs in the browser, `apps/api` on Node, and `packages/shared` holds types and JSON codecs that both use.
- [ ] **12.9** `test: plain elm still builds independent members`.
- [ ] **12.10** `release: 1.0`.

**Exit:** the example workspace builds, tests and serves under `nomi run --watch`, independent members still build with plain elm, and the language server works across members.

## Risks

| Risk | Mitigation |
|---|---|
| Error-message parity is the longest pole: `Syntax.hs` is 5,904 lines, `Type.hs` 1,619 and `Canonicalize.hs` 1,258. | Titles and regions gate first, and message text ratchets to 100% by 0.2. |
| Formatter parity stalls on edge cases. | The formatter tree mirrors elm-format's AST, and rule 6 applies. |
| Upstream ships changes, such as 0.19.4 or server modules through Acadia. | One Compat per release; server code lives only in packages and kernels. |
| Scale: upstream is about 95k lines across the three tools. | Releases 0.1, 0.2 and 0.3 each replace one tool on their own. |
| Sessions don't share memory. | The plan, the tests and the history carry all state. |
| Some oracles need GHC. | Only tier 3 uses them; their snapshots are checked in. |
