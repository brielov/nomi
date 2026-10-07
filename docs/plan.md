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
- [ ] **1.17b** `syntax: elm-format parity`. The elm-format columns reach 100%, with one commit per root cause. Two kinds of work:
  - elm-format's legacy syntax: operator definitions such as `(=>) = …`, operators with `$`, `#`, `@`, `~` or Unicode symbols, `\x` escapes and `[a..b]` ranges;
  - its indentation model, which checks only application arguments (`indented`) and block alignment (`checkIndent`), where elm checks every continuation.
- [ ] **1.17c** `syntax: GLSL validation`. elm parses each shader with language-glsl and rejects what it can't parse (`Parse/Shader.hs`). Port the subset that decides accept/reject; 1,894 mutations of the shader seed turn on it.
- [ ] **1.18** `syntax: fuzz target and benchmarks`.
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

- [ ] **2.1** `oracle: formatter snapshots`. Hashes of elm-format 0.8.8's output on the corpus. Import elm-format's own `tests/test-files`, the old fixtures, the bug fixtures and the 119 tuple cases. **Check:** the ratchet exists, at 0 passing.
- [ ] **2.2** `format: Box layout engine`. A port of `Box.hs` and `ElmStructure.hs`, starting from the salvaged `document.rs`. **Check:** unit tests mirroring elm-format's behavior.
- [ ] **2.3** `format: formatter tree`. Mirrors elm-format's AST (`AST/V0_16.hs` and its neighbors), with C1/C2 comment slots and multiline flags. Types only.
- [ ] **2.4** `format: lower module header and imports`. One forward comment cursor, plus a debug assertion that every comment is used exactly once.
- [ ] **2.5** `format: lower types and patterns`.
- [ ] **2.6** `format: lower expressions`.
- [ ] **2.7** `format: lower declarations`.
- [ ] **2.8** `format: render header, imports and exposing`. Import normalization; exposing lists grouped by `@docs`. **Check:** the ratchet climbs, as in every render step.
- [ ] **2.9** `format: render types and patterns`.
- [ ] **2.10** `format: render expressions`.
- [ ] **2.11** `format: render declarations and top-level spacing`.
- [ ] **2.12** `format: normalize literals`. Numbers and string/char escapes, from the salvaged `literal.rs`.
- [ ] **2.13** `format: doc comments`. Markdown normalization, and Elm code blocks through the 1.14 entry points. Decide at the start of the step, with differential fixtures, between porting Cheapskate's relevant parts and using pulldown-cmark.
- [ ] **2.14** `format: check invariants on corpus and proptest`. Four invariants:
  - formatting is idempotent;
  - the output parses;
  - every comment survives;
  - the token stream is unchanged apart from elm-format's normalizations (5.13 upgrades this to "HIR unchanged").
- [ ] **2.15** `format: reach corpus parity`. Work the ratchet to 100%, with one commit per root cause. Deviations are allowed only for crashes, non-idempotence, meaning changes or unparseable output, and each gets a fixture and a `DECISIONS.md` entry.
- [ ] **2.16** `format: linear time and depth safety`. The 136, 274 and 549 KB benchmark scales linearly, and 5,000 nested expressions format.
- [ ] **2.17** `engine: VFS and thread pool`. Reads, atomic writes, file discovery, and the process's only pool. This is pulled forward from M4 so the `format` crate never touches files.
- [ ] **2.18** `cli: nomi format`. Paths, `--check` and `--stdin`, run in parallel through the engine. `fmt` is an alias.
- [ ] **2.19** `cli: elm-format personality`. Under the name `elm-format`: flags, prompts, messages, `--validate` JSON and exit codes. **Check:** transcript snapshots against elm-format 0.8.8.
- [ ] **2.20** `release: 0.1`. README usage (`ln -s nomi elm-format`), benchmarks, and tag `v0.1.0`.

**Exit:** byte parity on the corpus, the invariants hold on the corpus and under fuzzing, time is linear, and the elm-format CLI transcripts are identical. You review the deviation list and try it in your editor; editors call `elm-format --stdin`.

## M3: Projects and packages

Goal: read and write `elm.json` exactly as elm does, resolve and fetch dependencies, and provide `init` and `install`.

- [ ] **3.1** `project: elm.json model`. Application and package outlines, with parse errors classified as upstream does them, and re-serialization byte-identical to elm's. **Check:** every corpus `elm.json` round-trips; error fixtures, including the `elm.json` cases from the old CLI report matrix (see 7.5).
- [ ] **3.2** `project: names, versions and constraints`, with upstream's validation rules.
- [ ] **3.3** `project: Compat and paths`. `Compat` comes from `elm-version`. Root discovery works as upstream's does. A single module computes every nomi path.
- [ ] **3.4** `registry: package index`. `/all-packages` plus incremental `/all-packages/since/N`, usable offline. HTTP sits behind a trait, and tests use a fixture registry.
- [ ] **3.5** `registry: package store`. Downloads `endpoint.json` and the zip, verifies the hash, extracts with hardening, downloads in parallel, and stores by content.
- [ ] **3.6** `oracle: solver snapshots`. Rebuild the old Haskell solver oracle (`conformance/oracles/package-solver/Main.hs`) over a frozen index; this may need GHC 9.8 (D3). Scenarios come from the old solver tests plus generated ones, including cases with several valid answers and conflicts that force backtracking, and `install`'s relaxation order. It comes before the solver so its selection policy is tested from the start. **Check:** the ratchet exists, at 0 passing.
- [ ] **3.7** `registry: pubgrub solver`. Must select what upstream selects, not just a valid answer. Upstream's `Deps/Solver.hs` takes pending packages in name order (`Map.minViewWithKey`), tries versions newest first, and keeps the first success of a depth-first search; for `install`, the relaxation order in `Install.hs`. Failures come with an explanation. If PubGrub's priorities can't reproduce that selection on the 3.6 cases, select with a port of upstream's search and keep PubGrub for explanations, with a `DECISIONS.md` entry.
- [ ] **3.8** `registry: reach solver parity`. Work the ratchet to 100%.
- [ ] **3.9** `cli: new and add`. `nomi new` (alias `init`) and `nomi add` (alias `install`), plus the `elm init` and `elm install` personalities: prompts, the printed plan, and `elm.json` edits. **Check:** transcript snapshots recorded from elm.
- [ ] **3.10** `cli: rm`. Removes a direct dependency and re-solves, so indirect dependencies nothing else needs disappear too. Test dependencies are handled the same way. Alias `remove`. **Check:** fixture-registry scenarios; the result is what `elm install` would produce for the remaining direct dependencies.
- [ ] **3.11** `cli: update, outdated and tree`. `update` re-solves for the newest versions the constraints allow, optionally for one package and the packages that depend on it, and prints the plan before writing. `outdated` and `tree` only read. **Check:** fixture-registry scenarios.

**Exit:** solver parity, byte-identical `elm.json` rewrites, and every corpus app's dependencies resolve and fetch.

## M4: Engine, reports and syntax errors (with LSP v0)

Goal: `nomi check` reports syntax and module errors exactly as `elm make` does, and the language server runs on the same engine.

- [ ] **4.1** `engine: revisions, overlays and FileIds`. A versioned VFS with editor overlays, and FileIds canonical across symlinks (R7).
- [ ] **4.2** `engine: module graph`. Crawls the way upstream's `Build.hs` does: module ↔ path, packages' exposed modules, and the not-found, ambiguous, name-mismatch and cycle errors.
- [ ] **4.3** `engine: scheduler and cache`. The module DAG runs on the pool, with cancellation between phases, a self-trimming content-addressed per-user cache, and keys built from interface hashes.
- [ ] **4.4** `base: report documents and renderers`. A port of `Reporting/Doc.hs`, `Render/Code.hs` and `Report.hs`, with terminal and `--report=json` renderers and regions that depend on Compat.
- [ ] **4.5** `oracle: syntax error report snapshots`. Full JSON reports for the mutation suite, for each Compat.
- [ ] **4.6** `syntax: upstream error contexts`. For the first error, the parser records upstream's error constructor and context, which gives the title and region. **Check:** the title-and-region ratchet.
- [ ] **4.7** `report: syntax errors in headers and imports`. The first part of `Reporting/Error/Syntax.hs` (5,904 lines). **Check:** the full-report ratchet climbs, as in 4.8 to 4.10.
- [ ] **4.8** `report: syntax errors in declarations and types`.
- [ ] **4.9** `report: syntax errors in expressions and patterns`.
- [ ] **4.10** `report: syntax errors in literals, strings, numbers and shaders`.
- [ ] **4.11** `report: module and import errors`, the matching parts of `Reporting/Exit.hs`.
- [ ] **4.12** `cli: nomi check with elm make's reporting policy`. Only the earliest failing phase per module is shown, and blocked modules stay silent.
- [ ] **4.13** `lsp: v0`. New `ide` and `lsp` crates.
  - The `lsp-server` loop and position encodings.
  - Overlays and syntax diagnostics.
  - Formatting as a minimal diff.
  - Document symbols.

  **Check:** transcript tests on the salvaged harness.

**Exit:** syntax error titles and regions are identical on the mutation suite for every Compat, and the message-text ratchet is tracked. `nomi lsp` works in your editor.

## M5: Names (canonicalization)

- [ ] **5.1** `compiler: HIR and source map`. Arenas with IDs, lowered from the typed views, depth-safe.
- [ ] **5.2** `compiler: interfaces`. Values, unions, aliases and binops, with stable serialization and hashing.
- [ ] **5.3** `test: hermetic package fixtures`. Pinned copies of the elm/* packages the corpus apps use, with their licenses, so tier 1 compiles real programs offline.
- [ ] **5.4** `compiler: import environment`. Default imports; qualified, unqualified and aliased imports; exposing lists; kernel imports; ambiguity.
- [ ] **5.5** `compiler: local scopes and duplicates`.
- [ ] **5.6** `compiler: canonicalize types and aliases`.
- [ ] **5.7** `compiler: canonicalize patterns and expressions`.
- [ ] **5.8** `compiler: operator precedence`. Turns flat chains into trees using the imported fixities.
- [ ] **5.9** `compiler: top-level SCCs and recursion checks`.
- [ ] **5.10** `compiler: ports, effects and exposing checks`.
- [ ] **5.11** `oracle: naming error snapshots`. Curated cases plus mutations (renames, removals from exposing lists, duplicates), for each Compat.
- [ ] **5.12** `report: naming errors`. A port of `Reporting/Error/Canonicalize.hs` (1,258 lines) and `Reporting/Suggest.hs`.
- [ ] **5.13** `format: check the HIR-unchanged invariant`. Replaces the token-level check from 2.14.
- [ ] **5.14** `ide: definition, references and highlights`.

**Exit:** every corpus package and app each Compat accepts (1.15a) canonicalizes cleanly, naming errors match, and navigation works in the editor.

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

