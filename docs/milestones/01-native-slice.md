# M1 — Native vertical slice

Blueprint for milestone 1 of `docs/architecture.md` (§14 M1). It conforms to the
architecture plan; "A§x" points there. "POC" is `../algorithm-w`; "port X" means copy the
POC file into the named nomi path and apply exactly the listed changes.

## 1. Goal

`nomi build hello/` compiles this program through every stage into a static executable
that runs on a growable fiber stack, prints two numbers and exits 0:

```elm
module Main exposing (main)

import Stdio
import Task exposing (Task)


type Tree
    = Leaf
    | Node Tree Int Tree


build : Int -> Tree
build depth =
    if depth == 0 then
        Leaf

    else
        Node (build (depth - 1)) depth (build (depth - 1))


mapTree : (Int -> Int) -> Tree -> Tree
mapTree f tree =
    case tree of
        Leaf ->
            Leaf

        Node l v r ->
            Node (mapTree f l) (f v) (mapTree f r)


sumTree : Tree -> Int
sumTree tree =
    case tree of
        Leaf ->
            0

        Node l v r ->
            sumTree l + v + sumTree r


main : Task Never ()
main =
    Stdio.print (String.fromInt (sumTree (mapTree (\v -> v * 2) (build 20))) ++ "\n")
        |> Task.andThen (\_ -> Stdio.print (String.fromInt (List.foldl (+) 0 (List.map (\x -> x * 3) (List.range 1 100000))) ++ "\n"))
```

Expected stdout `4194260\n15000150000\n`, exit status 0.

## 2. Scope

**Builds** (A§14 M1): workspace and D24 build; project loading with the embedded `lib/`;
syntax, names, types, patterns for the subset of §4.3; Core with monomorphization,
direct-style `Task` and allocation sites; Flat with closure conversion, Perceus, borrow
inference and reuse for constructors, lists and closures; LIR; the x86 backend with the
M1 calling convention, stack checks, polls, the kernel, shim and inline sequences and the
machine-IR verifier; `nomi-rt` (§4.12); `nomi_config`, `nomi_globals`, `nomi_sites`
(`nomi_hash_seed` arrives with its first reader in M4); the M1 modules of A§5.11a; both `String` layouts and the leaf kernels
`fromInt`, `append`, `eq`, `compare`; `layout.rs` and `tests/layout.rs`; first versions
of `docs/grammar.md`, `docs/semantics.md`, `docs/diagnostics.md`; `tests/programs/`
with the slice's tests and harness; the reuse report; the first scoreboard rows.

**Does not build**: records, Float, Char literals, tuples in source (Core uses them
internally), `Debug`, `Dict`, any interpreter (M3, M4), `nomi check --machine` (M2), the
rest of Elm's layout rule (M2), the scheduler, `Process`, channels, io_uring (M6), the
IRC data-structure rewrite (M4, A§5.8), free-list trimming (A§6.1 at scheduler switches,
M6; page return M4), `--heap-audit` (M4), `--deterministic` and `--force-yield` (M6).
The CLI rejects those three flags with a usage error naming the milestone (exit 2).

## 3. Current state

- This repository: `Cargo.toml` (package `nomi`, no dependencies), `src/main.rs` (cargo
  template), `.gitignore` (`/target`), `docs/direction.md`, `docs/architecture.md`
  (blueprint final), this file. No commits. No `docs/conventions.md` or logs.
- POC (verified by reading): pipeline entry points `syntax::lex`/`parse`
  (`src/syntax/lexer.rs:18`, `parser.rs:702`), `ast::lower` (`ast/lower.rs:658`),
  `types::infer` (`types/infer.rs:683`), `matching::check::check` (`matching/check.rs:161`,
  mutates `Typed`, returns nothing), `core::lower::lower(ast, &mut Typed, &mut Interner,
  file, diags) -> Option<Program>` (`core/lower.rs:946`), `core::opt::optimize`
  (`core/opt.rs:45`; opt 0 does nothing, SAT only at opt 2), `flat::lower::lower(p,
  interner, file, diags) -> Option<FProgram>` (`flat/lower.rs:1290`), `flat::opt::optimize`
  (`flat/opt.rs:31`; CPR only at opt ≥ 2, reuse before borrow), `lir::lower::lower(p,
  rc_check, heap_limit_mib)` (`lir/lower.rs:764`), `lir::opt::optimize` (`lir/opt.rs:36`),
  `x86::isel::select` (`x86/isel.rs:1579`), `x86::regalloc::allocate`
  (`x86/regalloc.rs:700`), `x86::emit::emit` (`x86/emit.rs:271`), link with copy fallback
  on `CrossesDevices` (`driver.rs:589-676`). Types: `TyNode` 12 bytes with an `eq` flag on
  variables (`types/mod.rs:19-43`, `unify.rs:60-100`); `Span { start, end }` without file
  (`source.rs:8-11`); `Diagnostic` with string codes (`diag.rs:20-27`); Core `CoreTy
  {Int,Bool,Unit,Tuple,Fun}` (`core/mod.rs:120-127`), `Rhs` 12 bytes (`:294`), atoms with
  a 2-bit tag where `11` holds unit/false/true (`:42-92`); Flat `Kind {Scalar, Tuple,
  Closure, Token}` (`flat/mod.rs:33`), `Init`/`PrintMain`/`Release` functions synthesized in
  `flat/lower.rs:1352-1424`; LIR `Inst` 16 bytes (`lir/mod.rs:296`), `FreeMem` with children
  already handled; x86 GPR-only MIR, `extra_args` (`isel.rs:763-780`), `r11` as indirect
  call target and jump-table scratch (`isel.rs:785-790`, `x86/mod.rs:456-458`), naive
  allocator scratch `r10`/`r11` with implicit-operand handling (`naive.rs:61-100`), IRC
  K = 15 (`regalloc.rs:12-30`); runtime `src/x86/runtime.s` (889 lines).
- Verified on this host: `/tmp` is tmpfs and `/home` another filesystem; `TMPDIR` unset;
  a nested `cargo build` in `build.rs` must clear `RUSTC_WRAPPER`,
  `RUSTC_WORKSPACE_WRAPPER`, `CLIPPY_ARGS` or `cargo clippy` lints the runtime (A D24).

## 4. Design

### 4.1 Workspace and build

```
Cargo.toml                 [workspace] members = ["crates/nomi", "crates/xtask", "crates/nomi-algo"], exclude = ["crates/nomi-rt"], resolver = "2"
.cargo/config.toml         [alias] xtask = "run -p xtask --"
crates/nomi/Cargo.toml     package nomi; [lib] path src/lib.rs; [[bin]] nomi src/main.rs; dependency nomi-algo (path)
crates/nomi/build.rs       runtime archive and embedded lib/ (below); includes src/hash.rs by #[path]
crates/nomi-algo/          #![no_std], no alloc, no panic handler: pub fn fmt_i64(v: i64, buf: &mut [u8; 20]) -> usize (index of the first digit; digits in buf[i..20])
crates/nomi-rt/Cargo.toml  own [workspace]; [lib] crate-type = ["staticlib"]; profiles dev and release: panic = "abort", opt-level = 3, debug-assertions = false; dependency nomi-algo (path)
crates/xtask/              subcommands scoreboard, size
lib/                       Elm packages (§4.13), present from CP1 (empty until CP4)
```

The root `src/main.rs` is deleted.

`crates/nomi/build.rs`:
1. Runs `$CARGO build --release --manifest-path $CARGO_MANIFEST_DIR/../nomi-rt/Cargo.toml
   --target-dir $CARGO_MANIFEST_DIR/../../target/rt`, removing `RUSTFLAGS`,
   `CARGO_ENCODED_RUSTFLAGS`, `CARGO_TARGET_DIR`, `RUSTC_WRAPPER`,
   `RUSTC_WORKSPACE_WRAPPER`, `CLIPPY_ARGS` from its environment; panics on failure.
2. Writes `$OUT_DIR/rt_archive.rs`: `pub static RT_ARCHIVE: &[u8] =
   include_bytes!("<abs path of libnomi_rt.a>"); pub const RT_HASH: u64 = 0x…;` (FxHash
   of the bytes, `hash.rs` included by `#[path = "src/hash.rs"] #[allow(dead_code)] mod
   hash;`).
3. Walks `$CARGO_MANIFEST_DIR/../../lib` in sorted path order and writes
   `$OUT_DIR/lib_files.rs`: `pub static LIB: &[(&str, &[u8])] = &[("nomi:lib/<rel>",
   include_bytes!("<abs>")), …];`.
4. `cargo:rerun-if-changed=` for `../nomi-rt/src`, `../nomi-rt/Cargo.toml`,
   `../nomi-algo/src`, `../nomi-algo/Cargo.toml`, `../../lib`; `rerun-if-env-changed=` for
   the six removed variables.

Foundation files ported from the POC:
- `hash.rs`, `symbol.rs`, `alloc.rs`, `guard.rs` unchanged; `par.rs` with `AWC_JOBS`
  renamed `NOMI_JOBS` and `crate::driver::BIG_STACK` (defined in `driver/mod.rs` from CP1 as
  `1 << 30`). `main.rs` runs every command on a thread with a `BIG_STACK` stack after
  `guard::install_hook` and `guard::set_stack_base`; a `StackLimit` unwind becomes an ICE
  (exit 70).
- `source.rs`: per-file arrays (tokens, CST spans, AST span vectors) store
  `TextRange { start: u32, end: u32 }`; each file keeps `line_starts: Vec<u32>`, and line
  and column are computed by binary search when needed. `Span { file: FileId, start, end }`
  (A§6.15) appears only in diagnostics and cross-file tables (`Program.sites`); embedded
  files get `FileId`s with paths `nomi:lib/…`.
- `diag.rs`: `Diagnostic { code: u16, severity: Severity, primary: (Span, String),
  secondary: Vec<(Span, String)>, message: String, notes: Vec<String> }` (A§7.2 with a
  primary label and the POC's notes; amendment recorded in A); warnings never change the
  exit status;
  the renderer is the POC's, with paths relative to the project directory (`nomi:lib/…`
  for embedded files), 1-based lines and columns counted in Unicode scalars. Example:
  ```
  error[E0401]: expected `Int`, found `String`
   --> src/Main.elm:7:15
    |
  7 |     add 1 "two"
    |           ^^^^^ this is a `String`
    |
    = note: the second argument of `add` is an `Int`
  ```

### 4.2 Projects (`project/`)

```rust
pub struct Project { pub modules: Vec<ModuleSource>, pub packages: Vec<Package>, pub interner: Interner }
pub struct ModuleSource { pub name: ModuleName, pub file: FileId, pub package: PackageId, pub cst: Cst, pub imports: Vec<Import> }
pub struct Package { pub name: String, pub version: String, pub exposed: Vec<ModuleName>, pub prelude: Option<Vec<PreludeItem>>, pub deps: Vec<PackageId> }
pub fn load(dir: &Path, sm: &mut SourceMap, diags: &mut Diagnostics) -> Option<Project>;
```

- Reads `dir/elm.json`: `"type": "application"`, `"elm-version": "0.19.3"`,
  `"source-directories"`, `"dependencies": {"direct": {…}, "indirect": {…}}`; unknown
  fields ignored.
- Dependencies resolve to embedded packages `nomi:lib/<author>/<name>/<version>/`.
- Module discovery from module `Main`: parse each file once (the CST is kept in
  `ModuleSource`), follow its imports; `import Elm.Kernel.X` is allowed only in `nomi/*`
  modules and names no file. Modules are stored in topological order; `ModuleId` is the
  index.
- Prelude (A§5.1): from the `nomi/core` dependency's `prelude` field; modules of
  `nomi/core` get no default imports.
- Errors: E0101 `elm-version` not `0.19.3`; E0102 dependency not found; E0103 `elm/*`
  dependency (I8); E0104 module not found; E0105 import cycle; E0106 module name does not
  match its path; E0107 module defined in two places; E0108 import of a module the
  package does not expose; E0109 malformed `elm.json` (with the JSON error position);
  E0110 no `Main` module.

### 4.3 Syntax (`syntax/`)

Port `src/syntax/{mod,kind,lexer,parser,cst}.rs`; changes:

- **Tokens**: lower and upper identifiers; qualified names (`A.B.c`, `A.B.C`); integer
  literals (decimal, `0x` hex) checked to fit i64 (E0206); strings with `\n \r \t \" \'
  \\ \u{H…}` escapes (a surrogate or a value above `10FFFF` is E0203) and `"""` multiline
  strings; operator symbols (maximal runs of `+-*/<>=!&|^%.:?`); the runs `=`, `|`, `:`,
  `->`, `..` and the `.` of qualified names are reserved punctuation, not operators;
  `(`, `)`, `[`, `]`, `,`, `\`, `_`; keywords
  `module exposing import as type alias port effect where if then else case of let in
  infix`; comments `--`, `{- -}` (nesting), `{-| -}`; whitespace and newlines. Tokens
  store `TextRange`; the layout rule takes columns from the file's `line_starts`.
- **Unary minus** (Elm's rule): a `-` immediately followed by an expression-start token
  (no whitespace) and preceded by whitespace, `(`, `,`, `[`, an operator or the start of
  the line is negation of the following atom (`-f x` is `(negate f) x`, Elm 0.19); it
  lowers to the intrinsic `Basics.negate`.
- **CST**: the POC's preorder struct of arrays without `parent`. Binary operators are kept
  as flat operand/operator chains; `ast/` re-associates them by fixity (§4.4), so the
  parser needs no fixities (amendment of A§5.2's Pratt wording, recorded in A).
- **Layout** (`docs/grammar.md` version 1): a top-level declaration starts at column 1
  and any token at column 1 ends the current declaration; `let` opens a block at the
  column of its first binding, each binding starts at that column, `in` ends the block at
  any column; `case … of` opens a block at the column of its first pattern, each branch
  starts there, a token left of it ends the block; inside a binding or branch, tokens
  continue the expression while their column is greater than the block's. A token that
  breaks these rules gives E0204.
- **Grammar subset**:
  - module header `module M exposing (…)`; `port module` and `effect module` headers are
    E0205;
  - imports with `as` and `exposing (…)`;
  - `type T a … = C1 t… | C2 t…`; `type alias A a = t` (expanded by `types/`);
  - type expressions: `type := btype ("->" type)?`, `btype := UpperName atype* | atype`,
    `atype := lowerName | UpperName | "(" ")" | "(" type ")"`;
  - annotations `name : type`; declarations `name pat* = expr`; `infix left|right|non N
    (op) = name`;
  - expressions: integer and string literals, `()`, (qualified) variables and
    constructors, application, operator chains, operator sections `(+)`, negation, `if`,
    `let … in`, `case … of`, `\pat* -> expr`, `[e, …]`, parentheses,
    `Elm.Kernel.M.x`;
  - patterns: `_`, variables, `()`, constructors with argument patterns, integer
    literals, `[]`, `p :: p`, parentheses.
- `parse(file: FileId, src: &str, diags: &mut Diagnostics) -> Cst`; never fails.
- Errors: E0201 unexpected token (`expected <what>, found <token>`), E0202 unterminated
  string, E0203 invalid escape, E0204 layout, E0205 unsupported header, E0206 integer out
  of range.

### 4.4 AST and names (`ast/`)

Port `src/ast/{mod,lower,dump}.rs` (not `eval.rs`):
- `pub struct Asts { pub modules: Vec<ModuleAst>, pub expr_base: Vec<u32>, pub def_base:
  Vec<u32> }`; per-module arenas with local ids; global `ExprId(u32)`/`DefId(u32)` =
  base + local (A§6.15). The interner lives in `Project`.
- `lower_modules(project: &mut Project, diags: &mut Diagnostics) -> Asts` (A§7.1 with the
  CSTs owned by the project).
- Resolution of imports, `exposing`, qualified names, aliases and the prelude; errors
  E0301 shadowing (Elm's rule, checked against the oracle: a local binding may not reuse
  the name of an enclosing local binding or of a top-level definition of the same module;
  imported and prelude names may be shadowed), E0302 duplicate
  definition, E0303 unknown name (`cannot find <name>`), E0304 kernel reference outside
  `nomi/*`, E0306 name not exposed by its module, E0310 `Main` has no `main`.
- Operator chains re-associate by the fixities of `infix` declarations (Elm's algorithm);
  equal precedence with conflicting associativity is E0305.
- Binding groups: Tarjan's SCC over top-level and `let` definitions.

### 4.5 Types (`types/`)

Port `src/types/{mod,infer,unify}.rs`; changes:
- `TyNode` (12 bytes, `const _: () = assert!(size_of::<TyNode>() == 12)`, as in the POC):
  `Var { level: u32, link: u32, constraint: Constraint /*u8*/ }`,
  `Con { def: TypeId, args: u32 /*start in lists*/, len: u16, generic: bool }`,
  `Fun { param: TyId, ret: TyId, generic: bool }`, `Error`. The POC's `Int`, `Bool`,
  `Unit`, `Tuple` nodes become `Con`. `TypeId(u32)` is global and dense: ids 0–9 are the
  fixed constants `INT`, `BOOL`, `UNIT`, `STRING`, `LIST`, `TASK`, `NEVER`, `TUPLE2`,
  `TUPLE3`, `TASKOUT`; declared types are numbered from 10 as `type_base[m] + local`. The
  kernel table's type rows bind their declarations to the fixed ids, and those
  declarations take no `type_base` slot. The declarations of `Int`, `String`, `List`,
  `Task` and `Never` are placeholders: they contribute no constructors (a reference to
  `JustOneMore` or another placeholder constructor is E0303). `Bool`'s row keeps its
  constructors. Constructors get a global dense `CtorId(u32)`: fixed ids `False` 0,
  `True` 1, `()` 2, `[]` 3, `::` 4, `Ok` 5, `Err` 6, `Cancelled` 7, the 2-tuple 8, the
  3-tuple 9 (A§6.2's `Bool` order whatever the declaration order); declared constructors
  from 10 as `ctor_base[type] + index`, with `ctor_type: Vec<TypeId>` and
  `ctor_arity: Vec<u16>` (copied into `core::Program`).
- The `eq` flag, `make_eq` and `UnifyError::NotEq` are removed; function equality is a
  build-time error (A§1.1).
- `Constraint { None, Number, Comparable, Appendable, CompAppend }`: meeting two
  variables' constraints uses `Number ∧ Comparable = Number`, `Comparable ∧ Appendable =
  CompAppend`, `CompAppend ∧ Comparable = CompAppend`, `CompAppend ∧ Appendable =
  CompAppend`, `Number ∧ Appendable` and `Number ∧ CompAppend` → E0402. Binding to a type
  checks membership: `Number` ∈ {Int}; `Comparable` ∈ {Int, String, List c, tuples of
  comparables}; `Appendable` ∈ {String, List a}; `CompAppend` ∈ {String, List c}.
- Annotations: rigid variables; names starting with `number`, `comparable`, `appendable`,
  `compappend` are constrained rigid variables.
- Kernel references: a fresh variable, constrained by the annotation (A§7.3).
- `Typed`: the global arena, per-module scheme tables, and instantiations recorded during
  inference as `(start, len)` per `ExprId` in visit order; at the end of `infer` one
  compaction pass writes the CSR `inst_start: Vec<u32>` (by global `ExprId`) and
  `inst_tys: Vec<TyId>` in `ExprId` order, with every type resolved through `find` and the
  defaults applied: `Number`, `Comparable` → `Int`; `Appendable`,
  `CompAppend` → `String`; unconstrained → `()`.
- `infer(asts: &Asts, project: &Project, diags: &mut Diagnostics) -> Typed`.
- Errors: E0401 mismatch (`expected <t1>, found <t2>`), E0402 constraint (`<t> is not
  <class>`), E0403 infinite type, E0404 annotation more general than the definition,
  E0405 `main` is not `Task Never ()`.

### 4.6 Patterns (`matching/`)

Port `src/matching/{mod,check,compile}.rs`. `Ctor` becomes `{ Con(CtorId), Int(i64),
Nil, Cons }` (`Bool` and `()` are `Con`); signatures come from constructor lists (`Bool`,
`()` and custom types complete; `Int` infinite; `{Nil, Cons}` for lists).
`check_matches(asts: &Asts, typed: &Typed, diags: &mut Diagnostics) -> Matches`, with
`Matches { exhaustive: Vec<bool> }` indexed by the case expression's global `ExprId`;
argument patterns of declarations and lambdas are checked like a one-arm `case` and
recorded under the lambda's or declaration body's `ExprId` (Elm rejects `\(Just x) ->
x`); `lower_core` uses `matching::compile` to build decision trees. Errors: E0501
non-exhaustive (`missing pattern <witness>`), E0502 redundant pattern (warning).

### 4.7 Kernel table (`kernel/table.rs`)

Rows keyed by (kernel module, name); type rows keyed by (package, module, type name).
Rust symbols are `k_<Module>_<name>` with the module name as written (`k_String_fromInt`).

| Key | Class | Scheme or signature | Lowering |
|---|---|---|---|
| type rows `nomi/core` `Basics.Int`, `Basics.Bool`, `Basics.Never`, `String.String`, `List.List`, `Task.Task` | type | — | `INT`, `BOOL`, `NEVER`, `STRING`, `LIST`, `TASK` |
| `Basics.add`, `sub`, `mul` | intrinsic | `number -> number -> number` | LIR `Bin` (wrapping) |
| `Basics.idiv` | intrinsic | `Int -> Int -> Int` | divisor 0 → `rt_crash("division by zero")`; `minInt // -1` = `minInt` |
| `Basics.negate` | intrinsic | `number -> number` | `Bin Sub 0 x` |
| `Basics.eq`, `neq` | intrinsic | `a -> a -> Bool` | `()`: `True`; `Int`, `Bool`, nullary-only types: `Cmp`; `String`: `k_String_eq`; custom types and lists: generated equality; function types: E0602 |
| `Basics.lt`, `le`, `gt`, `ge` | intrinsic | `comparable -> comparable -> Bool` | `Int`: `Cmp`; `String`: `k_String_compare`; lists: generated comparison |
| `Basics.append` | intrinsic | `appendable -> appendable -> appendable` | `String`: `k_String_append`; `List a`: the Elm function `List.append`, found by (package, module, name) |
| `Basics.and`, `or` | intrinsic | `Bool -> Bool -> Bool` | short-circuit: `a && b` lowers to `if a then b else False`, `a \|\| b` to `if a then True else b` |
| `List.cons` | intrinsic | `a -> List a -> List a` | constructor `::` |
| `Task.succeed`, `fail`, `andThen`, `map`, `mapError`, `onError` | intrinsic | Elm's `Task` types | Core templates (§4.8) |
| `String.fromInt` | leaf | `Int -> String`; result `fresh` | `k_String_fromInt` |
| `String.append` | leaf | `String -> String -> String`; `own, borrow`; result `fresh` | `k_String_append` |
| `String.eq` | leaf | `String -> String -> Bool`; `borrow, borrow` | `k_String_eq` |
| `String.compare` | leaf | `String -> String -> Int` (−1/0/1); `borrow, borrow` | `k_String_compare` |
| `Stdio.print` | system | `String -> Task x ()`, `x` unconstrained and never produced; `borrow` | task closure capturing the string; `k_Stdio_print` |

Runtime Rust entry points called with the kernel sequence: `rt_crash`,
`rt_immortalize_globals`, `rt_rc_check_init`, `rt_stats_init`. Assembly fiber-control
routines called directly: `rt_morestack`, `rt_park`. Shims (§4.11): `rt_alloc_slow`,
`rt_alloc_checked`, `rt_free`, `rt_free_checked`, `rt_rc_slow_inc`, `rt_rc_slow_dec`.
The verifier's Rust-symbol set is every `k_*` symbol plus the four `rt_*` entry points.
A reachable kernel reference without a row is E0601. A top-level definition whose body
is exactly a kernel reference (`add = Elm.Kernel.Basics.add`) is an alias: references to
it resolve to the kernel. A saturated use (as many arguments as the row's arity) lowers
directly; an unsaturated or first-class use is eta-expanded into a lambda, which is a
static closure when it captures nothing. `Basics.idiv` lowers to: divisor 0 →
`rt_crash`; divisor −1 → `0 − a` (wrapping, so `minInt // -1` = `minInt`, avoiding the
`idiv` trap); else `idiv`. The optimizer never folds or speculates `idiv` with a divisor
that is not a known nonzero constant.

### 4.8 Core (`core/`)

Port `src/core/{mod,lower,build,simplify,occur,contify,opt,lint,pretty}.rs`; `sat.rs`,
`wrapper.rs`, `accum.rs` and the unrolling passes move to M4 with the interpreters
(M1-D14). The M1 pipeline: opt 0 none; opt 1 the simplifier loop; opt 2 the simplifier
loop, contify, the simplifier loop. Every `FxHashMap`/`FxHashSet` keyed by a dense id in
the ported files (`core/build.rs:56-62` rewrites, `core/simplify.rs:86` `live_across`,
`core/simplify.rs:1143-1251` sets and `join_live`) becomes a `Vec` with a sentinel
(A§6.15). Unit tests written in the POC's ML syntax are rewritten as Elm sources or
deleted. Changes:
- `lower_core(asts: &Asts, typed: &Typed, matches: &Matches, project: &Project, main:
  DefId, diags: &mut Diagnostics) -> core::Program`; `program.ok` is false after errors.
- Types: `CoreTy { Int, Bool, Unit, Str, Con(TypeId, ListId), List(CTy), Fun(CTy, CTy),
  Tuple(ListId), TaskOut(CTy /*x*/, CTy /*a*/), Never }` (12 bytes, asserted),
  hash-consed in `TyInterner` with the POC's `(u8, u32, u32)` key and tags `Int` 0,
  `Bool` 1, `Unit` 2, `Str` 3, `Con` 4, `List` 5, `Fun` 6, `Tuple` 7, `TaskOut` 8,
  `Never` 9; argument lists are interned slices `ListId(u32)` (0 = empty) with a side
  `Vec<Range>` into `lists`.
  `TaskOut` has the built-in `TypeId` `TASKOUT` with constructors `Ok` (0, field `a`),
  `Err` (1, field `x`), `Cancelled` (2, nullary).
- `CoreTy` is canonical: the conversion from types maps each fixed `TypeId` to its
  dedicated variant (`INT` → `Int`, `LIST` → `List`, `TUPLE2/3` → `Tuple`, `TASK x a` →
  `Fun(UNIT, TaskOut(x, a))`, `NEVER` → `Never`); `TyInterner::intern` debug-asserts that
  `Con` never carries a fixed id, and the Core lint checks it.
- Atoms: tag `11` keeps payloads 0/1/2 for unit/false/true; payload `3 + CtorId` is a
  nullary constructor (no extra table). `Bool` constants are always `TRUE`/`FALSE`; the
  lint rejects `Bool` `CtorId`s in atoms. `AtomView` gains `Ctor(CtorId)`.
- `Rhs` (12 bytes kept): `Con(CtorId, u32 /*start in atoms; count = arity*/)`,
  `Field(Atom, u16 /*field*/)`, `Str(u32 /*string pool*/)`, `Kernel(KernelId, u32 /*start;
  count from the row*/)` added.
- Allocation sites: `Program.sites: Vec<Span>`; `site_of: Vec<u32>` indexed by `EId`
  for `Con`, `Lam`, `App` (it may build a partial application) and kernel calls,
  `NO_SITE = u32::MAX` elsewhere; the lint checks that every allocating form has a site.
  The Builder copies the entry with each copied expression, so inlined copies keep their
  site; a pass that synthesizes an allocation copies the site of the expression it
  replaces; template and eta-expanded lambdas take the site of the kernel reference;
  Flat's partial applications and task closures take the site of the Core `App` or
  kernel call; `w_main`'s runtime calls use site `nomi_sites.count` (the runtime entry).
  Core keeps sites in a side table because `Expr` is 20 bytes and `Rhs` has no room.
- String literals are deduplicated in the pool in first-occurrence order.
- `Case` arms are `CaseArm { key: u32, body }`: the constructor index for custom types,
  lists and `Bool`, the literal `Atom` bits for `Int` (the scrutinee's type decides);
  fields are bound by `Field` lets inside the arm.
- Monomorphization: an FxHash map keyed by (definition, interned `CTy` list `ListId`); the
  POC's caps and node budget (`core/lower.rs:182-194,351-357,972`) are removed. While
  instantiating group G at key K, an instantiation of a member of G at a key not equal to
  a key on G's instantiation stack is E0603.
- Generated equality and comparison: `Vec<VarId>` per family indexed by `CTy` (sentinel
  `NO_VAR`), grown as types are interned (replaces `eq_fns`, `core/lower.rs:53`). E0602 for
  `==` at a function type.
- **Task** (A§6.8): `Task x a` at a `CTy` is `Fun(UNIT, TaskOut(x, a))`. Templates:
  `succeed v = \() -> Ok v`; `fail e = \() -> Err e`; `andThen f t = \() -> case t () of
  Ok a -> f a (); Err e -> Err e; Cancelled -> Cancelled`; `map`, `mapError`, `onError`
  alike. `TaskOut` values may be let-bound, join parameters and results; the lint rejects
  a `TaskOut` stored in a constructor, closure capture or global.
- **Optimizer passes and the new forms**: the simplifier treats `Con`, `Str` and `Kernel`
  of leaf kernels as pure, `Field` as pure but not total (it never floats out of its
  arm), and does case-of-known-constructor; SAT, accum, contify and unrolling treat the
  new forms as opaque values; worker/wrapper unboxes only Core `Tuple`. Kernel calls of
  system kernels are effects and never move.
- Lint: port `core/lint.rs`, extended to `Field` (only inside an arm whose tag matches),
  `Con` arities, `ctor_consts`, `TaskOut` placement, `site_of` entries.

### 4.9 Flat (`flat/`)

Port `src/flat/{mod,lower,build,simplify,reuse,borrow,rc,rcopt,opt,lint,pretty}.rs`;
`cpr.rs` and `interp.rs` move to M4. `flat/reuse.rs:20-25,177-180` maps become `Vec`s;
`print_fns` (`flat/lower.rs:107`) is removed with printing. Changes:
- `lower_flat(p: &core::Program, diags: &mut Diagnostics) -> flat::Program` (amended
  A§7.1); E0701 more than 4094 fields; E0703 more than 248 constructors.
- **Kinds**: `Scalar` (Int, unit), `Imm` (Bool and nullary-only types), `Tuple(LayoutId)`
  (single-layout products: Core tuples, single-constructor multi-field types), `Sum(TypeLayoutId)`
  (custom types with at least one constructor with fields, and lists), `Closure`, `Str`,
  `Token`, `TaskPair` (a `TaskOut` value: two variables, status and payload). `Never` is
  `Scalar` (it has no values). A field may hold a pointer when its kind is `Tuple`, `Sum`,
  `Closure` or `Str`; `Scalar` and `Imm` fields are scalar fields.
- **Layouts** (A§6.1–6.2): `Layout { fields: Range, phys: Range, scan: u32, words: u32,
  tag: u8 }` per constructor, pointer fields first in declaration order; `TypeLayout {
  ctors: Range /*into ctor_layouts: Vec<u32>; LayoutId or NULLARY = u32::MAX per index*/,
  nullary: u32, boxed: u32 }`; memo `Vec<u32>` indexed by `CTy` (sentinel `u32::MAX`)
  giving a `LayoutId` for `Tuple` kinds and a `TypeLayoutId` for `Sum` kinds. A
  single-constructor single-field type takes its field's kind, unless the unwrap chain
  reaches the same type again, in which case it is a `Sum` with one boxed constructor.
  `List a`: `[]` immediate 1, `::` 3 words tag 1, `[hdr][head][tail]` when `a` may be a
  pointer, else `[hdr][tail][head]`. Nullary constructor k of a `Sum` is immediate
  `2k + 1`.
- **Closures**: `[hdr tag 0][info][heap captures][scalar captures]`, `scan = 1 + heap
  captures`; `info` is a `.rodata` object `[hdr rc 0 size 4 scan 0 tag 251][entry][arity]
  [fast]`; `entry` replaces the POC's `entry1` with the A§6.6 convention. A partial
  application is a closure capturing the applied closure and the arguments so far (the
  POC's curry stubs).
- **TaskOut**: every `TaskOut` value is a `TaskPair` of two variables in every position
  and at every opt level; functions returning it use `LetMulti`/two-value `Ret`; case on
  it is a switch on the status variable. Inc/Dec of a `TaskPair` payload branches on the
  status when `x` and `a` have different kinds.
- **Free and Reset** carry the layout of the enclosing arm: `FExpr::Free(FVar, LayoutId)`,
  `FRhs::Reset(FVar, LayoutId, kept)` (`kept` marks fields 0–31; higher fields are never
  marked kept). Drop specialization emits each child's `Dec` explicitly before `Free` only
  for layouts of at most 64 words; a larger object is dropped by a single `Dec`, whose
  zero path calls `rt_free`, which walks the children.
- **Allocation sites**: the allocating `FRhs` variants (`Tuple`, `Con`, `MkClosure`,
  `Reuse`) carry `site: u32` (`FRhs` is 24 bytes, asserted).
- **Static closures**: closures with no captures (top-level functions used as values,
  eta-expanded kernels) are `.rodata` objects `[hdr rc 0 size 2 scan 1 tag 0][info]`; the
  POC's `STATIC_RC` is replaced by rc 0.
- **opt 0** keeps the POC's path (RC insertion only, every parameter owned) except that
  kernel calls take their `borrow`/`own` modes from the kernel table at every level.
- **RC**: Perceus (`rc.rs`), borrow inference (`borrow.rs`) and RC optimizations
  (`rcopt.rs`) extended to `Imm` (no RC), `Sum` (RC operations carry `imm_guard` when the
  type has nullary constructors), `Str`, `Closure` (never decrement `info` inline).
- **Reuse** (`reuse.rs`): tokens from `Reset` of any `Tuple`/`Sum` constructor layout or
  closure may serve any allocation of the same `words`; `Reuse` writes the new header and
  fields. The pass order stays the POC's (reuse, borrow, rc, rcopt; CPR and simplify at
  opt 2).
- **Reuse report**: `reuse.rs` records an outcome per allocation into `outcomes: Vec<u8>`
  indexed by site (0 none yet, 1 hit, 2 `size`, 3 `live-after`, 4 `no-candidate`); a new
  outcome merges by keeping the larger code except that 1 never overrides a miss.
  Reasons: `size` — a `Reset` token was available in the function but none of the
  allocation's `words`; `live-after` — a token of the right size existed but its object is
  used after the allocation; `no-candidate` — no `Reset` token reaches the allocation.
  Sites whose only allocations are kernel calls are omitted from the report (kernels
  allocate on their own). `--report reuse` writes
  `<output>.reuse`, one line per site, `reuse <path>:<line>:<col> hit` or
  `reuse <path>:<line>:<col> miss <reason>`, sorted by (path, line, col, site id), paths
  project-relative or `nomi:lib/…`.
- **w_init and w_main**: `lower_flat` synthesizes `w_init` (the POC's `FuncKind::Init`)
  and `w_main` (`FuncKind::Main`); `FuncKind::PrintMain`, `FuncKind::Release` and
  `FExpr::PrintInt`/`PrintStr` are removed. `w_main` calls, in order:
  `rt_rc_check_init` (checked builds) and `rt_stats_init` (`--stats` builds) as kernel
  calls, `w_init`, the kernel call `rt_immortalize_globals`, then `main`'s task closure
  with unit; it ignores the result and returns (into `rt_fiber_exit`). M1 never builds a
  string index: every index word is the immediate 1.
- Lint: port `flat/lint.rs`, extended to the new kinds, `TaskPair` arity, `Free`/`Reset`
  layouts matching the scrutinized arm.

### 4.10 LIR (`lir/`)

Port `src/lir/{mod,lower,verify,cfg,dce,compact,dom,pretty,opt}.rs`; `basecall`,
`sccp`, `range`, `thread`, `gvn` and `licm` move to M4. The M1 LIR pipeline at opt ≥ 1:
`cfg`, `dce`, `cfg`; always `compact`. Changes:
- `lower_lir(p: &flat::Program, cfg: &LirConfig /*rc_check, stats*/) -> lir::Program`;
  the POC's `heap_limit_mib` is removed.
- `Inst` stays 16 bytes. New and changed instructions: `Alloc { dst, words: u16, site:
  u32, hdr_idx: u32 /*64-bit header in imms*/ }`; `Load8 { dst, base, offset: i32 }`;
`RcInc { v, n, imm_guard: bool }`,
  `RcDec { v, imm_guard: bool }`; `FreeMem { v, words }` (children already decremented,
  words ≤ 64); calls gain `CalleeKind::Kernel` with the kernel symbol in `Callee.id` and a
  `site` in `CallData`.
- `RtFn`: `AllocSlow` and `Free` kept; `AllocChecked`, `FreeChecked`, `RcSlowInc`,
  `RcSlowDec` added; `PrintInt`, `PrintStr`, `Flush`, `AllocLarge`, `FreeLarge`,
  `FreeNoChildren`, `RcFreedObject`, `DivZero` removed. `rt_crash` is a kernel callee.
- **Case on a `Sum`**: (1) `Bin And v, 1`; branch; (2) immediate side: one nullary
  constructor → its arm, else `Switch` on `v >> 1`; (3) pointer side: one boxed
  constructor → its arm, else `Load8 [v + 7]` and `Switch`.
- Field access `Load [v + 8·(1 + phys)]`; string literals `StrAddr` of a `.rodata` string
  object; `TaskPair` returns are two-value `Return`.
- Verifier: port `verify.rs`, plus: `Alloc` header `size` equals `words`; `imm_guard` set
  exactly on `Sum` values with nullary constructors; kernel calls name table symbols.

### 4.11 x86 backend (`x86/`)

Port `src/x86/*` except `runtime.s`; changes:

**Calling convention** (settles A§12's M1 delegation, M1-D1):
- Integer arguments `rdi, rsi, rdx, rcx, r8, r9`; arguments 7+ on the stack. At a call
  with m stack arguments the caller evaluates all arguments, then `sub rsp, 8m`, stores
  argument 7 + j at `[rsp + 8j]` (the emitter adds 8m to `rsp`-relative spill offsets
  between the `sub` and the `call`), loads the register arguments, and calls; the callee
  returns with `ret 8m`, so the caller's `rsp` is restored. On entry argument 7 + j is at
  `[rsp + 8 + 8j]`.
- Results `rax, rdx, rcx, rsi, rdi, r8`. Callee-saved `rbx, rbp, r12–r15`. From M4, float
  arguments `xmm0–xmm7`, float results `xmm0–xmm3`, all XMM caller-saved.
- `r10`, `r11` reserved: not allocatable (IRC K = 13, the POC's IRC otherwise unchanged);
  their only uses are the transient ones listed under the verifier.
- **Tail calls** from a function that received k stack arguments to one taking m: write
  the outgoing stack arguments to slots at the bottom of the frame (the frame reserves
  8m + 8·max(0, m − k) bytes for this, so every outgoing slot lies below every final
  destination), load the register arguments, restore callee-saved registers with `mov`
  from their save slots, `mov r11, [return-address slot]`, then with `r10` copy each
  outgoing slot j to `ret_new + 8 + 8j` in ascending address order, where
  `ret_new = rsp_entry + 8(k − m)`, store `r11` at `ret_new`, `add rsp, <ret_new − rsp>`
  and `jmp`. This covers m < k, m = k and m > k.
- Indirect tail calls: the closure is in `rdi` (first argument of `fast`/`entry`); after
  the copy above, `mov r11, [rdi + 8]; jmp [r11 + 24]` (or `+ 8` for `entry`).
- `extra_args` (`isel.rs:763-780`) is removed. IRC's color `ORDER` (`regalloc.rs:13-29`)
  loses `r10` and `r11`; the jump-table index keeps its interference with `r11`.
- New MInst `Movzx8Mem { dst, base, offset }` for `Load8`.

**Pseudo-instructions**: isel emits `AllocSeq`, `FreeSeq`, `IncSeq`, `DecSeq`,
`KernelCall`, `ShimCall`, `StackCheck`, `Poll` with declared clobbers; they are expanded
into the sequences below after register allocation, then the machine-IR verifier runs,
then emission. Clobbers: `AllocSeq` {rax, r10, r11, flags} plus its destination;
`FreeSeq`, `IncSeq`, `DecSeq`, `ShimCall` {rax, r10, r11, flags}; `KernelCall` all
caller-saved registers plus `r10`, `r11`, flags; `StackCheck`, `Poll` {r11, flags}.

**Sequences** with H(n) = `0x90 + 16·(n − 2)`, C(n) = H(n) + 8:
- `AllocSeq` n ≤ 64 into `d`: `[--stats: inc qword ptr fs:[0x38]]`; `mov d, qword ptr
  fs:[H]; test d, d; jz .Ls; mov r11, [d + 8]; mov qword ptr fs:[H], r11; dec dword ptr
  fs:[C]; .Lh: mov r11, <imm64 header>; mov [d], r11` with the cold block `.Ls: mov r11d,
  n; call rt_alloc_slow; mov d, rax; jmp .Lh`. For 64 < n ≤ 4094 only the cold sequence
  plus the header store. For n > 4094: `mov r10, (tag << 56) | scan; mov r11d, n; call
  rt_alloc_slow; mov d, rax` (the shim writes the large header).
- `FreeSeq` n ≤ 64: `mov r11, qword ptr fs:[H]; mov [p + 8], r11; mov qword ptr fs:[H],
  p; inc dword ptr fs:[C]`; otherwise `mov r11, p; call rt_free`.
- `IncSeq` (n ≥ 1): `[imm_guard: test v, 1; jnz .Ldone]; cmp dword ptr [v], 0; jle
  .Lslow; add dword ptr [v], n; jo .Lsat; .Ldone:` with cold blocks `.Lslow: je .Ldone;
  mov r11, v; mov r10d, n; call rt_rc_slow_inc; jmp .Ldone` and `.Lsat: mov dword ptr
  [v], 0; [checked: inc qword ptr [rip + rt_saturated]]; jmp .Ldone` (saturation makes the
  object immortal; checked reports print `saturated <n>`).
- `DecSeq`: `[imm_guard: test v, 1; jnz .Ldone]; cmp dword ptr [v], 1; jg .Ldec; je
  .Lfree; cmp dword ptr [v], 0; je .Ldone; mov r11, v; call rt_rc_slow_dec; jmp .Ldone;
  .Ldec: dec dword ptr [v]; jmp .Ldone; .Lfree: mov r11, v; call rt_free` (checked:
  `xor r10d, r10d; mov r11, v; call rt_free_checked`). Known-layout frees with explicit
  child decrements come only from Flat's drop specialization (an `IsUnique` branch, the
  children's `RcDec`s, then `FreeMem`).
- `KernelCall`: in checked builds `mov dword ptr fs:[0x40], <site>`, then the A§6.5
  sequence; system kernels add `cmp rax, 3; jne 1f; call rt_park; 1:` (M1's `rt_park`
  crashes with `internal error: pending kernel in M1`).
- Checked builds: `AllocSeq` becomes `[--stats: inc]; mov dword ptr fs:[0x40], <site>;
  mov r11d, n; call rt_alloc_checked; mov d, rax; mov r11, <imm64 header>; mov [d], r11`
  (n ≥ 4094: the large sequence, `r10` as above, no header store, because a checked
  object of n words needs n + 1); `FreeSeq` becomes `mov r10d, n; mov r11, p; call
  rt_free_checked`, and `rt_free_checked` crashes with `internal error: free size
  mismatch` when `r10` differs from `size(p)` (`r10` = 0 means "read the header", the
  path `rt_free` uses).
- `StackCheck`: `lea r11, [rsp − F]; cmp r11, qword ptr fs:[8]; jbe .Lm; .Lc:` with
  `.Lm: mov r11d, F; call rt_morestack; jmp .Lc` (after `rt_morestack` returns the
  check is satisfied, so it is not repeated). F = 8·(callee-saved pushes)
  + frame size + 8·(largest outgoing stack-argument count) + 8·(largest max(0, m − k) over
  tail calls). A function is checked when it contains any call or tail call (direct,
  indirect or kernel) other than a shim call, or a back-edge, or F > 256; every function
  that is the target of a direct tail call, and every closure `fast`/`entry` entry point,
  is checked.
- `Poll` on every back-edge: `cmp rsp, qword ptr fs:[8]; jbe .Lp` with `.Lp: xor r11d,
  r11d; call rt_morestack; jmp <back-edge target>`. Self tail calls jump to the function
  start and pass its `StackCheck`.
- Unknown calls with k arguments go through the POC's `apply_k` helpers
  (`lir/lower.rs:673-761`), generated for every k used: `apply_k(closure, a1..ak)` loads
  `info` and its `arity`; arity = k → tail-call `fast` (closure in `rdi`, arguments after
  it, stack arguments per the convention); arity > k → allocate a partial application
  closure; arity < k → call `fast` with the first `arity` arguments, then tail-call
  `apply_{k−arity}` on the result. There is no argument limit. Inside the helpers the
  indirect call is `mov r11, [rdi + 8]; call [r11 + 24]` (or `jmp` for tail calls).

Every static object (strings, closure `info`, static closures) is emitted with
`.p2align 3`. `Sym::ExtraArgs` (`x86/mod.rs:353`, `emit.rs:39`, the callee side at
`isel.rs:1543`) and `emit.rs`'s `include_str!("runtime.s")` with its `.if RC_CHECK`
handling (`emit.rs:271-290`) are removed.

**Naive allocator** (debug): scratch registers `rbx` and `r12`, saved by its prologue as
used callee-saved registers; the POC's implicit-operand handling (`naive.rs:61-65`) is
kept with these scratch registers.

**Machine-IR verifier** (`x86/verify.rs`, after expansion):
- I5: no `rsp`-derived value outside memory operands, except the `StackCheck` `lea r11`,
  the kernel sequence's `mov r11, rsp`, and the tail-call `add rsp`;
- I6: every call to a Rust symbol is inside a `KernelCall` expansion; shims only through
  `ShimCall` or the sequence expansions;
- I14: unchecked functions have no back-edge, no non-shim call and F ≤ 256; every
  back-edge has a `Poll`;
- I15: no register holding an `fs:`-loaded value is live across a call, poll, check or
  shim;
- `r10`/`r11` appear only in: the sequences above, `mov r11, [rdi + 8]` (or `[closure +
  8]`) immediately followed by its `call`/`jmp`, the jump-table scratch
  (`x86/mod.rs:456-458`), and the tail-call copy.

**Emission**: `.intel_syntax noprefix`; `.cfi_startproc`, `.cfi_def_cfa_offset`,
`.cfi_offset`, `.cfi_endproc` per function; functions in `.text.<symbol>`; static objects
in `.rodata.<symbol>` with rc 0; `nomi_config`, `nomi_globals`, `nomi_sites` in `.rodata`;
global slots in `.bss`.

Tool paths are `as` and `ld` unless `NOMI_AS`/`NOMI_LD` name others (used by the E0901 and
E0902 snapshot tests).

**Link** (`driver/link.rs`): create `$TMPDIR` (default `/tmp`) subdirectory
`nomi-<pid>-<nanos>-<counter>` with exclusive `mkdir` (mode 0700), removed by an RAII
guard on every path; write `prog.s` and `libnomi_rt.a`; run `as --64
-mx86-used-note=no -o prog.o prog.s` and `ld -static -nostdlib -z noseparate-code
--gc-sections -s -o prog prog.o libnomi_rt.a`; apply the POC's `drop_section_headers`
(`driver.rs:628-658`); `create_dir_all` the output's directory; copy `prog` to
`<out>.tmp-<pid>` there, set mode 0755, and `rename` it to `<out>` (same filesystem). On
failure the previous output is untouched. Errors E0901 (spawn), E0902 (nonzero exit, up
to 20 stderr lines as notes).

### 4.12 Runtime (`crates/nomi-rt`)

```
src/lib.rs        #![cfg_attr(not(test), no_std)]; panic_handler (cfg(not(test))) writes "runtime error: internal error: runtime panic at <file>:<line>" raw and exits 101
src/start.rs      cfg(not(test)): _start (global_asm!), boot, rt_enter, rt_fiber_exit, rt_park
src/tcb.rs        TCB offsets (A§6.4) and #[repr(C)] struct Tcb (one page)
src/stack.rs      stack classes and pool, rt_morestack entry (global_asm!) and slow path, canary
src/shims.rs      global_asm! shims: save rsp to fs:[0x28], switch to the system stack, save caller-saved GPRs and XMM0–15, call Rust, restore
src/heap.rs       chunks, refill, mid-size lists, span allocator, heap::alloc / heap::free (the Rust allocation API), rt_free with the pending stack, hooks
src/check.rs      --rc-check: rt_rc_check_init, checked alloc/free, poison, quarantine, live_by_site, report
src/stats.rs      rt_stats_init, the allocations report line
src/globals.rs    rt_immortalize_globals
src/io.rs         stdout buffer, k_Stdio_print, flush, report file writing, raw stderr
src/crash.rs      rt_crash, SIGSEGV handler, SIGPIPE ignore
src/kernels/string.rs  k_String_fromInt, k_String_append, k_String_eq, k_String_compare
src/layout.rs     dependency-free layout helpers (A§7.3), included by crates/nomi via #[path]
src/sys.rs        raw syscalls: read, write, openat, close, mmap, munmap, mprotect, madvise, arch_prctl, rt_sigaction, rt_sigreturn (SA_RESTORER trampoline), sigaltstack, poll, exit_group
src/mem.rs        cfg(not(test)): memcpy, memmove, memset, memcmp, bcmp (#[inline(never)], rep movsb/stosb)
```

**Boot** (`_start`): save the initial `rsp` (argc, argv, envp, read later for
`NOMI_REPORT_PATH`), switch to a freshly mapped system stack (8 MiB plus a guard page,
`MAP_NORESERVE`); install the alternate signal stack (64 KiB), `SIGSEGV`
(`SA_ONSTACK|SA_SIGINFO|SA_RESTORER`) and `SIGPIPE` (ignore); map and `arch_prctl` the
TCB; check `nomi_config` version 1 (else `runtime error: internal error: compiler and
runtime differ`, exit 101); allocate the main
fiber's stack (class for `stack_initial + R`) and enter `w_main` with `rt_enter`.
- **Fiber record** (128 bytes, the main fiber's in `.bss`): `saved_rsp`, `stack_lo`,
  `stack_class`, `real_limit`, `flags` (bit 0 main, bit 1 fresh), reserved words for M6.
- **rt_enter** for a fresh fiber: set `fs:[0x10]`, and `fs:[0x30]` = `fs:[8]` = limit (or
  `fs:[8]` = `u64::MAX` when `stack_move`), write the canary, place `rt_fiber_exit` as the
  return address at the top so that `rsp ≡ 8 (mod 16)` at entry, switch `rsp`, jump to the
  entry.
- **rt_fiber_exit** on the main fiber: switch to the system stack, flush stdout, run the
  exit hooks (checked, stats; they write the report and return the status, 0 or 105),
  `exit_group(status)`.
- **Stacks**: classes 1 KiB to 1 GiB; classes ≤ 64 KiB carved from 64 MiB pool mappings
  (`MAP_NORESERVE`, `madvise(MADV_NOHUGEPAGE)`) with a per-class free list, a new pool
  mapped when one is exhausted; larger classes get their own mapping; `mmap` failure →
  `out of memory`. No shrinking in M1. Canary: 16 bytes of `0x5A` at `limit − R`, checked
  under `--rc-check` at growth, release and exit (`stack canary overwritten`).
- **rt_morestack**: pushes the 15 GPRs other than `rsp`, the flags and XMM0–15
  (384 bytes), stores `rsp` into the fiber record, switches to the system stack and calls
  the slow path with F = `r11`. M1 has no preemption. When `rsp − F` is below the real
  limit, or when `stack_move` is set and the used part is at most 64 KiB: allocate the smallest class ≥ used + F + R
  (with `stack_move` a fresh region different from the last two, the released region
  filled with `0xDE`), `memcpy` the used part, rebase the saved `rsp`, write the canary,
  release the old stack, and set `fs:[0x30]` to the new limit and `fs:[8]` to it (or to
  `u64::MAX` while `stack_move` relocates); growth past 1 GiB → `stack overflow`. Then
  switch back, pop and return.
- **heap::alloc(words, scan, tag) -> *mut u64** and **heap::free(p)**: the Rust API used by
  kernels and `rt_free`; in checked builds (hook set by `rt_rc_check_init`) they take the
  checked paths with the site from `fs:[0x40]` (indexed as `min(site, nomi_sites.count)`);
  with the stats hook (set by `rt_stats_init`) a `heap::alloc` called from a kernel
  increments `fs:[0x38]` once. Shims never count; inline sequences count their own
  allocations.
- **No panicking operations**: `nomi-rt` uses no indexing with bounds checks, `unwrap`,
  `expect` or overflow-checked arithmetic in reachable code (explicit checks call
  `rt_crash` instead); a test links `hello` without `-s` and asserts that no symbol from
  `core::fmt` or `core::panicking` is present (measured: panic machinery costs about
  8 KB of the 16 KiB budget).
- **rt_alloc_slow(n)** (`r11` = n): n ≤ 64: refill the class by carving 64 objects of n
  words from the current chunk, pop one (count += 63); 64 < n ≤ 4094: geometric classes
  of sizes `2^k + j·2^(k−3)` words for k = 6..11 and j = 1..8 (72, 80, …, 128, 144, …,
  4096; 48 classes; at most 12.5% waste); the block is the smallest class ≥ n and the
  header keeps the exact `size`; pop that class's list (`HeapState.mid: [u64; 48]` in
  `.bss`), else bump the class size from the chunk;
  n > 4094: the span allocator (pages from regions of max(64 MiB, rounded size),
  address-sorted free-span list with coalescing), writing the large prefix and header
  (`rc 1`, size 0, scan 0, `tag` from `r10 >> 56`, scan prefix from `r10`'s low 32 bits).
  Chunks are 64 MiB `MAP_NORESERVE`. `mmap` failure → `out of memory`.
- **rt_free(p)** (`r11` = p): the POC's walk (`runtime.s:502-596`) in Rust: decrement the
  `scan(p)` children, skipping odd words and rc = 0 objects, continue with the last
  zero-count child, push the others on the pending stack (64 KiB segments from the span
  allocator; word 0 previous, word 1 cached next, entries from +16); release each object:
  `size(p) ≤ 64` → its free list (count += 1), 65–4094 → its geometric class, large → the
  span allocator. In checked builds release goes through the checked path, and a child
  whose size is 4095 crashes with `reference count operation on freed object`.
- **--rc-check** (`check.rs`, linked only through `rt_rc_check_init`): `rt_rc_check_init`
  allocates `live_by_site: [u32; nomi_sites.count + 1]` (the last entry for runtime
  allocations) from the span allocator and sets the hooks. `rt_alloc_checked(n)` takes a
  block of n + 1 words from class n + 1 (or the slow path), writes the prefix (site from
  `fs:[0x40]`, fiber id 1), returns `p` past it, increments `live_by_site[site]`.
  For n ≥ 4094 the checked object is large: the span allocator, prefix words at
  `p − 16` and `p − 8`, the site word at `p − 24`.
  `rt_free_checked(p)` rejects a header whose `size` is 4095 (`double free`), sets rc =
  `i32::MIN` and size = 4095, fills the fields with `0xDEAD_DEAD_DEAD_DEAC` (an even
  non-canonical address, so stale loads fault), decrements the prefix site's counter, and
  pushes an entry onto the 65,536-entry quarantine ring (`.bss` in a section referenced
  only from `check.rs`): `block | (words + 1) << 48` for small objects, `block | 1 << 63`
  for large ones (whose prefix words survive poisoning). Eviction returns a small block to
  class `words + 1` (count += 1) and a large one to the span allocator. `rt_rc_slow_inc/dec`
  crash with `reference count operation on freed object` when size = 4095. A reused object
  keeps the site of its original allocation, so alloc and free counts stay symmetric. The
  immortalization walk decrements each reached object's site
  counter and counts it as immortal. At exit: open `NOMI_REPORT_PATH` (failure → stderr
  `rc-check: cannot open report`, exit 105), write `live <sum>`, `immortal <n>`, one
  `leak <path>:<start>-<end> <count>` line per site with a nonzero counter in site order,
  and exit 105 if the sum is nonzero; without `NOMI_REPORT_PATH` the summary goes to
  stderr.
- **Reports**: one writer opens `NOMI_REPORT_PATH` once (`O_WRONLY|O_CREAT|O_TRUNC`,
  mode 0644) and writes, in order, `allocations N` (`--stats`), `live N`, `immortal N`,
  `saturated N` (`--rc-check`; the counter `rt_saturated` lives in `check.rs`), then the
  `leak` lines; the exit status is the larger of the two hooks' statuses; stderr gets a
  summary only on failure. `rt_rc_slow_inc/dec` on rc < 0 with size ≠ 4095 crash with
  `internal error: shared object in M1`.
- **rt_immortalize_globals()**: walks the slots of `nomi_globals` with an explicit
  segmented stack, skipping odd words and rc = 0 objects, and sets rc = 0 (no string
  index is built in M1).
- **Stdout**: 64 KiB buffer. `k_Stdio_print(s)` flushes first when the bytes do not fit,
  writes pieces of 64 KiB or more directly, and appends the rest; flushing uses `write(1)`
  loops: `EINTR` retried; `EAGAIN` waits with `poll`; `EPIPE` exits silently with status
  141; other failures → `write failed`. Returns status 0, payload 0.
- **Crash** (`rt_crash(msg, len) -> !`): unless a stdout write is in progress (TCB flag),
  flush stdout in writes of at most `PIPE_BUF` bytes, each after `poll(POLLOUT)` with the
  remaining part of a 100 ms budget, stopping at the first failed write or `POLLERR`/
  `POLLNVAL` and dropping the rest (file status flags are never changed; the crash flush
  never calls `rt_crash`);
  write `runtime error: <msg>\n` to fd 2; `exit_group(101)`. `SIGSEGV` in the system
  stack's guard page → `stack overflow`; otherwise `segmentation fault (internal error)
  at 0x…, rip 0x…`.
- **String kernels**: capacities of `k_String_fromInt` results and of literals are the
  byte length rounded up to whole words. `k_String_fromInt(v)` formats with
  `nomi_algo::fmt_i64` into a fresh string; `k_String_append(a, b)`: needed = len(a) + len(b) (more than 2^32 − 1 →
  `string too large`); if `a` has rc = 1 and capacity ≥ needed, append in place (writing
  index 1); otherwise allocate capacity `min(2 · needed, 2^32 − 1)` rounded to words
  (bounded by the 2^32 − 1 cap above),
  copy `a` then `b`, then decrement `a` (correct when `a` and `b` are the same object);
  `scalar_len` adds the count of non-continuation bytes appended. `k_String_eq` compares
  `byte_len` and bytes; `k_String_compare` compares bytes (scalar order).

### 4.13 `lib/`

`lib/nomi/core/1.0.0/elm.json`:
```json
{"type": "package", "name": "nomi/core", "summary": "nomi's core library", "license": "MIT",
 "version": "1.0.0", "exposed-modules": ["Basics", "List", "Maybe", "String", "Task"],
 "elm-version": "0.19.3", "dependencies": {}, "test-dependencies": {},
 "prelude": [{"module": "Basics", "as": null, "exposing": ".."},
             {"module": "List", "as": null, "exposing": ["List", "(::)"]},
             {"module": "Maybe", "as": null, "exposing": ["Maybe(..)"]},
             {"module": "String", "as": null, "exposing": ["String"]},
             {"module": "Task", "as": null, "exposing": ["Task"]}]}
```
CP4 ships this file with `"exposed-modules": ["Basics", "String", "Task"]` and a prelude
of `Basics`, `String`, `Task` only; CP7 switches to the full file above. Package
`elm-version` fields accept the range form and are not checked.
`lib/nomi/platform/1.0.0/elm.json`: the same shape with `"name": "nomi/platform"`,
`"exposed-modules": ["Stdio"]`, `"dependencies": {"nomi/core": "1.0.0 <= v < 2.0.0"}`, no
`prelude`.

Sources (each kernel-using module starts with `import Elm.Kernel.<M>`). Headers:
`module Basics exposing (Int, Bool(..), Never, add, sub, mul, idiv, negate, eq, neq, lt,
le, gt, ge, append, and, or, not, identity, always, apL, apR, composeL, composeR, (+), (-),
(*), (//), (==), (/=), (<), (>), (<=), (>=), (++), (&&), (||), (|>), (<|), (<<), (>>))`;
`module List exposing (List, (::), cons, range, map, foldl, foldr, reverse, append)`;
`module Maybe exposing (Maybe(..), withDefault, map)`; `module String exposing (String,
fromInt, append, eq, compare)`; `module Task exposing (Task, succeed, fail, andThen, map,
mapError, onError)`; `module Stdio exposing (print)`.
- `Basics.elm`: `type Int = Int`, `type Bool = True | False`, `type Never = JustOneMore
  Never`; `add`, `sub`, `mul`, `idiv`, `negate`, `eq`, `neq`, `lt`, `le`, `gt`, `ge`,
  `append` as kernel references with Elm's annotations; `infix right 0 (<|) = apL`,
  `infix left 0 (|>) = apR`, `infix right 2 (||) = or`, `infix right 3 (&&) = and`,
  `infix non 4 (==) = eq`, and `(/=)`, `(<)`, `(>)`, `(<=)`, `(>=)` with the same fixity
  (to `neq`, `lt`, `gt`, `le`, `ge`),
  `infix right 5 (++) = append`, `infix left 6 (+) = add` and `(-)`, `infix left 7 (*) =
  mul` and `(//) = idiv`, `infix left 9 (<<) = composeL`, `infix right 9 (>>) = composeR`;
  `identity`, `always`, `not`, `apL`, `apR`, `composeL`, `composeR` in Elm; `and`, `or`
  as kernel references (short-circuit intrinsics).
- `List.elm`: `type List a = List` (placeholder recognized by the type row),
  `cons = Elm.Kernel.List.cons`, `infix right 5 (::) = cons`;
  `range lo hi = rangeHelp lo hi []` with `rangeHelp lo hi acc = if lo <= hi then
  rangeHelp lo (hi - 1) (hi :: acc) else acc`; `foldl` tail-recursive;
  `map f xs = case xs of [] -> []; x :: rest -> f x :: map f rest` (direct recursion);
  `foldr f acc xs = case xs of [] -> acc; x :: rest -> f x (foldr f acc rest)`;
  `reverse`, `append`.
- `Maybe.elm`: `type Maybe a = Just a | Nothing`, `withDefault`, `map`.
- `String.elm`: `type String = String`, `fromInt`, `append`, `eq`, `compare` kernel
  references.
- `Task.elm`: `type Task x a = Task`, `succeed`, `fail`, `andThen`, `map`, `mapError`,
  `onError` as kernel references with Elm's annotations.
- `lib/nomi/platform/1.0.0/src/Stdio.elm`: `print : String -> Task x ()` as
  `Elm.Kernel.Stdio.print`.
- License MIT (M1-D24): the repository root `LICENSE` holds the MIT text (copyright
  "Gabriel Vaquer and the nomi contributors"); every crate's `Cargo.toml` has
  `license = "MIT"`; `lib/nomi/core/1.0.0/LICENSE` and `lib/nomi/platform/1.0.0/LICENSE`
  hold the same MIT text, and the core one appends elm/core's BSD-3-Clause notice for the
  adopted code; both packages' `elm.json` say `"license": "MIT"`.

### 4.14 Driver and CLI

`main.rs` parses arguments by hand. Defaults `--opt 2`, `--regalloc irc`, `--stack-initial
512`. Commands:
- `nomi check [DIR]`: project, syntax, names, types, patterns; prints diagnostics; exit 0
  (no errors) or 1.
- `nomi build [DIR] [-o PATH] [FLAGS]`: output `DIR/build/<directory name>` by default;
  `--emit tokens|cst|ast|types|core|core-opt|flat|flat-rc|lir|lir-opt|asm` prints that
  stage to stdout and does not link; `--report reuse` writes `<output>.reuse`.
- `nomi run [DIR] [FLAGS] [-- ARGS…]`: `build`, then executes the output with inherited
  stdio; exit status is the program's, or 128 + n when it was killed by signal n.
- `nomi interp`: usage error `error: nomi interp is available from M3` (exit 2).
- Flags: `--symbols` (keep symbols and section headers, for crash triage), `--opt 0|1|2`,
  `--regalloc naive|irc`, `--rc-check`, `--stats`,
  `--stack-initial N` (256 ≤ N ≤ 2^30 − 512, multiple of 16), `--stack-move`; `--heap-audit`,
  `--deterministic`, `--force-yield` give usage errors naming M4/M6.
- Usage errors print `error: <message>` and the synopsis to stderr, exit 2. `--help`
  prints the synopsis of A§5.9 and exits 0. ICEs print `error: internal compiler error:
  <text> at <location>` and exit 70.

### 4.15 Diagnostics (`docs/diagnostics.md`)

`docs/diagnostics.md` lists every code with its template and an example; each code has a
snapshot test `R/tests/diagnostics/<code>/` (an `elm.json`, sources, `expected.stderr`).
Templates (`<x>` are fills):

| Code | Message |
|---|---|
| E0101 | `this application declares elm-version <v>; nomi applications declare 0.19.3` |
| E0102 | `dependency <name> <version> was not found` |
| E0103 | `elm/* packages cannot be used by nomi build (<name>)` |
| E0104 | `module <M> was not found in the source directories or dependencies` |
| E0105 | `modules import each other in a cycle: <A> -> <B> -> … -> <A>` |
| E0106 | `module <M> is declared in <path>, which should declare <N>` |
| E0107 | `module <M> is defined in two places: <path1> and <path2>` |
| E0108 | `package <p> does not expose module <M>` |
| E0109 | `elm.json is malformed: <reason>` |
| E0110 | `no Main module in the source directories` |
| E0111 | `cannot read <path>: <os error>` |
| E0112 | `<path> is not valid UTF-8 (byte <offset>)` |
| E0201 | `expected <what>, found <token>` |
| E0202 | `this string is not terminated` |
| E0203 | `invalid escape sequence <text>` |
| E0204 | `this line is indented less than the <construct> it continues` |
| E0205 | `<port|effect> modules are not supported by nomi` |
| E0206 | `the integer <text> does not fit in 64 bits` |
| E0301 | `<name> is already defined in this scope` (secondary: the earlier definition) |
| E0302 | `<name> is defined twice` |
| E0303 | `cannot find <name>` |
| E0304 | `kernel references are only allowed in nomi packages` |
| E0305 | `operators <op1> and <op2> have the same precedence and conflicting associativity` |
| E0306 | `module <M> does not expose <name>` |
| E0310 | `module Main does not define main` |
| E0401 | `expected <t1>, found <t2>` |
| E0402 | `<t> is not <class>` (`a number`, `comparable`, `appendable`, `comparable and appendable`) |
| E0403 | `this would need the infinite type <t>` |
| E0404 | `the annotation says <t1>, but the definition is only <t2>` |
| E0405 | `main must be a Task Never (), but it is <t>` |
| E0501 | `this case does not cover <witness>` |
| E0502 | warning: `this pattern is never reached` |
| E0601 | `the kernel function <M>.<name> is not available` |
| E0602 | `== cannot compare functions (<t>)` |
| E0603 | `<name> is used at a growing type (<t>); polymorphic recursion is not supported by nomi build` |
| E0701 | `<ctor> has <n> fields; at most 4094 are supported` |
| E0703 | `<type> has <n> constructors; at most 248 are supported` |
| E0901 | `cannot run <tool>: <os error>` |
| E0902 | `<tool> failed with status <n>` (notes: up to 20 stderr lines) |
| E0903 | `cannot write <path>: <os error>` |

E0601 is tested by a unit test of `lower_core` (no shipped kernel lacks a row). E0103 also
applies to `nomi check` in M1; M2 lifts it for the parity harness. E0901 and E0902 are
provoked with `NOMI_AS`/`NOMI_LD`, E0903 with `-o` into a read-only directory.

## 5. Data layout

| Structure | Representation | Access | Why |
|---|---|---|---|
| Tokens, CST | the POC's, no `parent` | parser, AST lowering | A§6.15 |
| Instantiations | CSR by global `ExprId` | `lower_core` once per occurrence | dense ids, one allocation |
| Mono instances | FxHash map (DefId, interned `ListId`) → `VarId` | each polymorphic reference | no cap, fixed-size keys |
| Generated eq/compare | `Vec<VarId>` per family by `CTy`, `NO_VAR` | `==`, `compare` lowering | dense ids |
| Layouts | `Vec<Layout>`, `Vec<TypeLayout>`, `ctor_layouts: Vec<u32>`, memo `Vec<u32>` by `CTy` | Flat lowering, LIR offsets | dense ids |
| Allocation sites | `Vec<Span>` plus `site_of: Vec<u32>` by `EId`, carried into Flat and LIR | reuse report, `nomi_sites`, checked builds | one table for all three uses |
| Runtime objects | A§6.1–6.2 | generated code, runtime | contract |
| TCB | one page per worker | every allocation, free, check | fixed `fs:` offsets |
| Mid-size free lists | geometric classes (8 per power of two) in a `.bss` `HeapState` | `rt_alloc_slow`, `rt_free` | bounded waste; freed blocks serve nearby sizes |
| Pending stack | 64 KiB segments | `rt_free`, immortalization | no recursion (I18) |
| Quarantine | ring of 65,536 `u64` (`block \| words << 48`, or `block \| 1 << 63` for large) | checked frees | release information kept after poisoning |
| `live_by_site` | `[u32]` by site id | checked alloc/free, report | leak report by position |

## 6. Invariants

| Invariant | Check in M1 |
|---|---|
| I1 | lossless round trip of every `.elm` in `lib/`, `tests/programs/`, `tests/diagnostics/`, and of every char-boundary prefix of them |
| I2 | Core lint, Flat lint, LIR verifier, machine-IR verifier after every pass in debug builds |
| I3, I4 | `--rc-check` runs; `live 0` required when `expected.status` is 0; poisoned rc and size; quarantine |
| I5, I6, I14, I15 | machine-IR verifier; unit tests with hand-built violating functions for each rule |
| I7 | `tests/layout.rs` for `String` (both layouts) and the kernel-use type test for every row |
| I8 | an `elm/core` dependency gives E0103 |
| I9 | E0304 test |
| I10 | every test program built twice in two fresh temporary directories, bytes compared |
| I11 | E0701 and E0703 tests |
| I16 | canary check under `--rc-check` |
| I18 | `rt_free` and immortalization use explicit stacks; `bigdrop` frees a 10^6-element list |

## 7. Verification

**Harness** `crates/nomi/tests/programs.rs` (`harness = false`): for each
`tests/programs/<name>/` (files `elm.json`, `src/Main.elm`, `expected.stdout`,
`expected.stderr`, `expected.status`, optional `flags`):
- all paths are relative to the repository root `R = $CARGO_MANIFEST_DIR/../..`
  (`R/tests/programs`, `R/tests/diagnostics`, `R/target/programs`);
- builds with `-o R/target/programs/<name>/<config>/prog` in four configurations named
  `naive0` (`--opt 0 --regalloc naive --rc-check --stack-initial 256`), `irc1` (`--opt 1
  --rc-check --stack-initial 256`), `irc2` (`--opt 2 --rc-check --stack-initial 256`) and
  `prod` (`--opt 2`);
- runs each with `NOMI_REPORT_PATH=…/<config>/report`, `RLIMIT_AS` = 4 GiB (set with
  `setrlimit` in `pre_exec`), a 60 s deadline (the process group is killed), stdout and stderr drained concurrently with a 16 MiB cap each; compares
  stdout, stderr and status; for checked runs with expected status 0 the report must say
  `live 0`; programs run on `available_parallelism` threads, except `deep7`, which runs
  alone;
- then one `--stack-move` pass at `--opt 2 --rc-check --stack-initial 256` for programs
  without the flag `skip_stack_move`;
- flags (A§9.3): `native_only`, `skip_naive` (not built with `naive0`),
  `skip_stack_move`.

**Programs**: every `Main.elm` is `module Main exposing (main)` with the imports it uses
(`Stdio`, `Task exposing (Task)`, plus nothing else: `List`, `Maybe`, `String` are in the
prelude); programs that use `Tree`, `build`, `mapTree` or `sumTree` copy them verbatim
from §1, and programs that use `b2s` define `b2s : Bool -> String` as `b2s b = if b then
"T" else "F"`. Each printed line is one `Stdio.print`, chained with `|> Task.andThen (\_ ->
…)` in the order listed.

| Name | Program | Expected stdout / status |
|---|---|---|
| `hi` | `main = Stdio.print "hi\n"` | `hi\n` / 0 |
| `hello` | §1 | `4194260\n15000150000\n` / 0 |
| `fib` | naive `fib n = if n < 2 then n else fib (n - 1) + fib (n - 2)`, print `fib 25` | `75025\n` / 0 |
| `tree` | `build 16`, `mapTree (\v -> v + 1)`, `sumTree` | `196589\n` / 0 |
| `lists` | print `foldl (+) 0 (range 1 1000)`, `foldr (\x acc -> x - acc) 0 (range 1 10)`, `foldl (+) 0 (append (range 1 3) (range 4 6))`, `foldl (+) 0 (map (\x -> x * x) (range 1 10))` | `500500\n-5\n21\n385\n` / 0 |
| `strings` | build `s` by appending `String.fromInt i ++ ","` for i = 1..10000 in a tail-recursive loop (48,894 bytes, past the large-object size), print `s ++ "\n"`; print `fromInt (-9223372036854775807 - 1)`, `fromInt 0`, `fromInt 9223372036854775807`, each followed by `"\n"`; print `(let ab = "ab" in ab ++ ab) ++ "\n"` and `(let twelve = String.fromInt 12 in twelve ++ twelve) ++ "\n"` | the concatenation and newline, then `-9223372036854775808\n0\n9223372036854775807\nabab\n1212\n` / 0 |
| `divzero` | `zero = 0` at top level; print `"before\n"`, then print `String.fromInt (1 // zero)` (the optimizer never folds or moves `//` by a zero divisor, §4.7) | stdout `before\n`, stderr `runtime error: division by zero\n` / 101 |
| `minint` | `m = -9223372036854775807 - 1` and `d = -1` at top level; print `fromInt (m // d)` and `fromInt (m * d)` | `-9223372036854775808\n-9223372036854775808\n` / 0 |
| `deep` | `count n = if n == 0 then 0 else 1 + count (n - 1)`; print `count 1000000` | `1000000\n` / 0 |
| `deep7` | the same with `count 10000000`; flags `native_only`, `skip_naive`, `skip_stack_move` | `10000000\n` / 0 |
| `bigdrop` | `let xs = List.range 1 1000000 in case xs of [] -> "empty"; x :: _ -> String.fromInt x` printed with `"\n"` (the head is read, the whole list is then dropped by one `Dec` whose `rt_free` walks 10^6 cells), then the same for `build 20` dropped after a borrowed `sumTree`; flag `skip_stack_move` | `1\n` then `` `sumTree (build 20)` `` = `2097130\n` / 0 |
| `args` | `f8` with 8 parameters returning `Σ k·aₖ`, `f12` with 12; print `f8 1 2 3 4 5 6 7 8` and `f12 1 … 12`; `apply8 h a b c d e f g i = h a b c d e f g i` (an unknown call through `apply_8`), print `apply8 f8 1 2 3 4 5 6 7 8`; `g = f12 1 2 3 4 5` at top level (a partial application), print `g 6 7 8 9 10 11 12`; `flag = True` at top level, `pick b = if b then f8 else \_ _ _ _ _ _ _ _ -> 0`, `callIt h = h 1 2 3 4 5 6 7 8` (an indirect tail call), print `callIt (pick flag)` | `204\n650\n204\n650\n204\n` / 0 |
| `sums` | `type Shape = Circle Int \| Rect Int Int \| Tri Int Int Int \| Empty \| Dot`; `area`: `Circle r` → `r * r`, `Rect w h` → `w * h`, `Tri a b c` → `a + b + c`, `Empty` → 0, `Dot` → 1; `morph`: `Circle r` → `Rect r r`, `Rect w h` → `Circle (w + h)`, `Tri a b c` → `Tri c b a`, `Empty` → `Dot`, `Dot` → `Empty`; `shapes = [ Circle 1, Rect 2 3, Tri 4 5 6, Empty, Dot, Circle 7 ]`; print `foldl (+) 0 (map area shapes)`, `foldl (+) 0 (map (\x -> area (morph x)) shapes)`, `foldl (+) 0 (map area (map (\n -> Rect n 1) (range 1 10)))`; `type Big = Big Int … Int` (70 `Int` fields, an object above 64 words), `big = Big 1 2 … 70`, print the sum of its fields by one `case` | `72\n91\n55\n2485\n` / 0 |
| `crossreuse` | `type W = W Int`; `ws = List.map (\x -> W (3 * x)) (List.range 1 100)`; print `foldl (+) 0 (map (\(W v) -> v) ws)`; then `ys = List.map (\(W v) -> v // 3 + 1) ws`, print `foldl (+) 0 ys` (list cells reused across element types) | `15150\n5150\n` / 0 |
| `tailcalls` | `keep n = if n == 0 then 0 else 1 + keep (n - 1)`; `t7 n a b c d e g = if n == 0 then a+b+c+d+e+g else let k = keep 2 in t10 (n - 1) (a + k) b c d e g 1 2 3`; `t10 n a b c d e g x y z = if n == 0 then a+b+c+d+e+g+x+y+z else let k = keep 2 in t12 (n - 1) a (b + k) c d e g x y z 4 5`; `t12 n a b c d e g x y z p q = if n == 0 then a+…+q else if n - (n // 3) * 3 == 0 then t12 (n - 1) a b (c + 1) d e g x y z p q else let k = keep 2 in t7 (n - 1) (a + p) (b + q) c (d + k) e g`; print `t7 3000 1 2 3 4 5 6` | `15021\n` / 0 |
| `equality` | print one line, `List.foldr (++) "" (List.map b2s [ … ])` over: `"abc" == "abc"`, `"abc" == "abd"`, `"" == ""`, `"ab" < "abc"`, `"b" > "abc"`, `"é" > "z"`, `range 1 3 == range 1 3`, `range 1 3 == range 1 4`, `range 1 3 < range 1 4`, `build 3 == build 3`, `build 3 == mapTree (\v -> v + 1) (build 3)`, `Leaf /= Leaf`, `True == False` | `TFTTTTTFTTFFF\n` / 0 |
| `tasks` | `Task.fail 1 \|> Task.onError (\e -> Task.succeed (e + 1)) \|> Task.map (\v -> v * 10) \|> Task.andThen (\v -> Stdio.print (String.fromInt v ++ "\n"))`, then `Task.fail 7 \|> Task.mapError (\e -> e * 2) \|> Task.onError (\e -> Stdio.print (String.fromInt e ++ "\n"))`; with `flag = True` at top level, `p1 : Bool -> Task String Int`, `p1 b = if b then Task.succeed 3 else Task.fail "no"`, then `p1 flag \|> Task.map (\v -> v * 2) \|> Task.onError (\_ -> Task.succeed 0) \|> Task.andThen (\v -> Stdio.print (String.fromInt v ++ "\n"))`; `p2 : Bool -> Task Int String`, `p2 b = if b then Task.fail 9 else Task.succeed "x"`, then `p2 flag \|> Task.onError (\e -> Task.succeed (String.fromInt e)) \|> Task.andThen (\v -> Stdio.print (v ++ "\n"))` | `20\n14\n6\n9\n` / 0 |
| `operators` | with `b2s b = if b then "T" else "F"` and `zero = 0`, print each followed by `"\n"`: `5 \|> (\x -> x + 2)`, `(\x -> x + 2) <\| 5`, `((\x -> x + 1) >> (\x -> x * 2)) 5`, `((\x -> x + 1) << (\x -> x * 2)) 5`, `b2s (True \|\| (1 // zero == 0))`, `b2s (False && (1 // zero == 0))`, `b2s (not False)` | `7\n7\n12\n11\nT\nF\nT\n` / 0 |
| `layouts` | print each followed by `"\n"`: `List.foldr (++) "" [ "a", "b" ]`; `String.fromInt (List.foldl (+) 0 (List.map sumTree [ build 1, build 2 ]))`; `List.foldr (++) "" (List.map b2s [ True, False ])`; for `type Box = Box Bool (Maybe Int) Tree` and `bx = Box True (Just 3) (build 2)`, `case bx of Box f m t -> b2s f ++ " " ++ String.fromInt (Maybe.withDefault 0 m) ++ " " ++ String.fromInt (sumTree t)`; with `ns = [ 1, 2, 3 ]`, `List.foldr (++) "" (List.map String.fromInt ns)` and `String.fromInt (List.foldl (+) 0 (List.map (\x -> x + 1) ns))` | `ab\n5\nTF\nT 3 4\n123\n9\n` / 0 |
| `stackmove` | `count 2000` (non-tail) and `List.map (\x -> x * 2) (List.range 1 2000)` summed | `2000\n4002000\n` / 0 |
| `andthen/chain`, `andthen/base` | `chain`: `main = Task.succeed () \|> Task.andThen (\_ -> Task.succeed ())` repeated 100 times in the source, then `\|> Task.andThen (\_ -> Stdio.print "ok\n")`; `base`: `main = Stdio.print "ok\n"` | `ok\n` / 0 each |

`hello`, `deep` and `tree` carry `skip_stack_move` (deep stacks make every check copy);
`stackmove` covers deep non-tail frames under stack moves.

The harness additionally builds `andthen/chain` and `andthen/base` at `--opt 2 --stats` and
asserts `allocations(chain) − allocations(base) ≤ 1`; and builds `hello` with `--opt 2
--report reuse` and requires a `hit` line whose position lies inside the definition of
`mapTree` in `src/Main.elm` and one inside `map` in `nomi:lib/nomi/core/1.0.0/src/List.elm`
(definition ranges found by name in the sources); `crossreuse` at `--opt 2 --report reuse`
has no `miss` line inside `List.map`. Reuse and the `andthen` property are
checked at `--opt 2` only; `--opt 0` and `--opt 1` must produce the same output.

**Other tests**:
- Diagnostics snapshots per code (§4.15).
- Unit tests: lexer, parser, layout rule cases; name resolution and fixity; constraint
  table; defaulting; mono and E0603; layout computation per §4.9 row; LIR verifier; x86
  verifier rules; calling-convention cases; `nomi-rt` allocator, span, pending-stack and
  quarantine logic under `cfg(test)`; `nomi_algo::fmt_i64` against `i64::to_string` for
  10^6 random and boundary values.
- `crates/nomi/tests/layout.rs`: for strings of byte lengths 0, 1, 7, 8, 9, 32,744 (4,093
  words of bytes), 32,752 and 100,000, builds the value through the compiler's literal
  emission (`x86/emit.rs` string objects) and through `layout.rs`
  (`pub fn str_object(bytes: &[u8]) -> Vec<u64>`, `pub fn str_words(byte_len: u32) ->
  u32`, `pub const STR_TAG_SMALL: u8`, `pub const STR_TAG_LARGE: u8`, the header and
  index-word helpers of A§6.2) and compares the words.
- Kernel-use type test (`kernel/table.rs` unit test): for every row, the declaration in
  the embedded `lib/` is inferred and its scheme must equal the row's type; the row's
  ownership modes must have one entry per parameter.
- Jobs equality: every program in `tests/programs/` is compiled with `--emit core-opt` and
  `--emit asm` at `NOMI_JOBS` = 1, 2 and 4; the outputs must be identical.
- `R/tests/rt/<name>.s` plus `<name>.expected` (three sections `--- stdout`, `--- stderr`,
  `--- status` followed by the decimal status); checked mode is selected by byte 4 of the
  program's `nomi_config` (1 = checked), and `_start` then calls `rt_rc_check_init`. Test
  names: `prologue_recursion`, `crash`, `guard_page`, `stack_move_regs`,
  `alloc_classes`, `alloc_classes_checked`, `use_after_free`, `shim_regs_<shim>`,
  `free_chain`:
  hand-written assembly programs defining `w_main`, `nomi_config` (16 bytes), `nomi_globals`
  and `nomi_sites` (count 0), linked against `RT_ARCHIVE` by `crates/nomi/tests/rt.rs`
  (CP2, CP3): 10^6 checked-prologue recursions; `rt_crash`; the system-stack guard page;
  `stack_move` on every check with register values compared before and after; 10^6
  allocations and frees per class under checked and production builds; a use-after-free
  through `rt_rc_slow_dec` on a poisoned object; per shim (`rt_alloc_slow` for n ∈ {2, 64,
  65, 4095}, `rt_alloc_checked`, `rt_free`, `rt_free_checked`, `rt_rc_slow_inc`/`dec`) all
  13 other GPRs and XMM0–15 filled with distinct patterns, the slow path forced, and
  compared after the call; a 10^6-cell chain freed by one `rt_free` call.
- `crates/nomi/tests/archive.rs`: builds `nomi-rt` into `R/target/rt-check` with the six
  D24 variables cleared and compares its FxHash with `RT_HASH`. `cargo test` of the runtime
  uses `--target-dir R/target/rt-test`.
- I10: every test program built twice, in two fresh temporary directories and with
  `NOMI_JOBS=1` and the default, bytes compared; the POC's worker-count equality test
  (`core/simplify.rs:1991`) is ported.
- `cargo test` of the runtime: `cargo test --manifest-path crates/nomi-rt/Cargo.toml
  --target-dir R/target/rt-test`, and clippy on it with the same `--target-dir`.
- No-panic test: `hello` linked with `--symbols` contains no symbol whose demangled name
  starts with `core::fmt` or `core::panicking`.
- `cargo xtask size`: `hello` built at `--opt 2` must be ≤ 16,384 bytes.

## 8. Performance workloads

- **Native data slice** (A§10.1): `bench/slice/tree/` (`sumTree (mapTree (\v -> v * 2)
  (build 20))`) and `bench/slice/lists/` (`foldl (+) 0 (map (\x -> x * 3) (range 1
  100000))`, repeated 100 times), with C references `bench/slice/{tree,lists}.c` that build
  the same structures with `malloc`, compute the same values and `free` them
  (`gcc -O2 -static`).
- **Scoreboard** (`cargo xtask scoreboard`): builds `target/release/nomi` first, then for
  each workload appends one row to `docs/scoreboard.md`:
  `| date | commit | workload | nomi_ms | c_ms | ratio | size_bytes | allocations | nomi_rss_kib | c_rss_kib | compile_ms |`.
  Times are the median of 5 runs after one warm-up; `allocations` from a `--stats` build;
  the RSS columns are `ru_maxrss` of the child (`wait4`) in the median run;
  `compile_ms` the median of 3 `nomi build --opt 2`.
- Budgets: lexing and parsing O(n); new Flat and LIR passes linear; `rt_free` O(objects
  freed).

## 9. Decision Log

| # | Decision | Alternatives | Why | Status |
|---|---|---|---|---|
| M1-D1 | Calling convention of §4.11 (stack arguments with callee pop, `r10`/`r11` reserved, K = 13, tail-call copy order, XMM rules for M4) | keep `extra_args` | A§6.6 | OPEN |
| M1-D2 | Refill by carving 64 objects; mid-size per-size lists; no trimming in M1 | trimming to overflow lists | trimming needs scheduler switches and page return (A amendment) | OPEN |
| M1-D3 | Stack classes 1 KiB–1 GiB, 64 MiB pools for classes ≤ 64 KiB, no shrinking | per-stack mappings | A§6.4, spike S5 | OPEN |
| M1-D4 | Reuse report lines `reuse <pos> hit` / `miss <reason>` in `<output>.reuse` | JSON | grep-friendly, no crates | OPEN |
| M1-D5 | Scoreboard row per workload with fixed metric columns | row per metric | A§12 constraint | OPEN |
| M1-D6 | `nomi-rt` at `opt-level = 3` in every profile | follow the outer profile | stable size and stack use | OPEN |
| M1-D7 | `TaskOut` as a two-variable `TaskPair` in Flat at every opt level | relying on CPR at opt 2 | the A§6.6 ABI holds at every level | OPEN |
| M1-D8 | Naive allocator scratch `rbx`/`r12` | `r10`/`r11` (POC), `rax`/`rdx` | reserved registers; implicit operands of `idiv` and calls | OPEN |
| M1-D9 | Tracer-bullet checkpoint order (§11) | layer by layer | exercises the ported backend on new forms early | OPEN |
| M1-D10 | `List.map` and `foldr` by direct recursion | `reverse (foldl …)` | local reuse at opt ≥ 1 without SAT; growable stacks make depth cheap | OPEN |
| M1-D11 | IRC rewrite in M4 | in M1 | no M1 benefit; less risk (A amendment) | OPEN |
| M1-D12 | Stdout `EPIPE` exits silently with 141 | crash 101 | conventional pipe behaviour (`prog \| head`) | OPEN |
| M1-D13 | `skip_naive`, `skip_stack_move` test flags | per-program depth changes | the naive allocator's frames and the stack-move cost are per-configuration | OPEN |
| M1-D14 | Optimizer passes beyond the simplifier and the RC passes treat the new forms as opaque in M1 (§4.8, §4.10) | extending every pass now | M4 brings the differential oracles that localize optimizer bugs | OPEN |
| M1-D15 | Geometric mid-size classes | one list per exact size | bounded waste with doubling appends | OPEN |
| M1-D16 | Allocation sites inline in Flat's allocating `FRhs` | a side table | no copy rule to keep in four passes | OPEN |
| M1-D17 | Per-file `TextRange` spans, `line_starts` for lines and columns | full `Span` per token | smaller hot arrays for M2's throughput target | OPEN |
| M1-D18 | Selective port: only the passes M1's pipelines run (§4.8–§4.10); SAT, wrapper, accumulator, unrolling, CPR, basecall, SCCP, range, threading, GVN and LICM in M4 | port everything and leave it unused | unused passes would need the new forms; M4 brings their oracles | OPEN |
| M1-D19 | Fixed `CtorId`s 0–9 and placeholder declarations without constructors | per-module ids for built-ins | runtime and layouts can name `[]`, `::`, `Ok`/`Err`/`Cancelled` statically | OPEN |
| M1-D20 | Unknown calls through generated `apply_k` helpers, no argument limit | a fixed maximum arity | the POC's scheme; no user-visible limit | OPEN |
| M1-D21 | No panicking operations in `nomi-rt`, checked by a symbol test | `#[cold]` panic paths | panic machinery costs about 8 KB of the 16 KiB budget (measured) | OPEN |
| M1-D22 | One report file writer for `--stats` and `--rc-check` with a fixed key order | one file per hook | one open, deterministic output for tests | OPEN |
| M1-D23 | `Diagnostic.primary` carries a label | a bare span | every snapshot shows what the caret means (A amendment) | OPEN |
| M1-D24 | License MIT, with elm/core's BSD-3-Clause notice kept for adopted code | BSD-3-Clause; MIT OR Apache-2.0; copyleft | the user's choice | LOCKED |

## 10. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Extending the ported optimizer to constructors breaks its invariants | lints after every pass; the four-configuration matrix from CP4 on |
| `andThen` chains allocate after inlining | the `andthen` check; if it fails, CP8 adds a Core rule run before the simplifier's size check and exempt from the inlining budget: `andThen (\x -> e) t`, with `t` a known task (a template application or a lambda), rewrites to `\() -> case t () of Ok x -> e (); Err e1 -> Err e1; Cancelled -> Cancelled` |
| Hello world above 16 KiB once the runtime is complete | the no-panic rule (§4.12, about 8 KB measured) and `xtask size`; CP3 links the runtime with a minimal `w_main` and measures; if over 16 KiB, the largest functions in the link map that only crash paths reach move into `.text.unlikely` sections before CP4 |
| `rt_morestack` misses a register | the `stack_move` assembly test (CP2) and the `--stack-move` pass |
| `deep7`'s naive frames exceed 1 GiB | `skip_naive` on `deep7`; `deep` covers the naive configuration |
| The selective port leaves a dangling reference to a deferred pass | the ported `opt.rs` pipelines list only the passes of §4.8 and §4.10; `cargo build` fails on any reference to a file not ported |

## 11. Checkpoints

Every checkpoint ends with `cargo build`, `cargo test`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo test` and `cargo clippy -- -D warnings` of
`crates/nomi-rt` with `--target-dir R/target/rt-test` green.

**CP1 — Workspace and foundations.** Files per §4.1, `crates/nomi/src/{lib.rs,main.rs}`
(CLI skeleton with `--help`), the ported foundation files, `crates/nomi-algo` with
`fmt_i64` and its tests, `crates/nomi-rt` skeleton (`lib.rs`, `sys.rs`, `mem.rs`, a
`_start` that writes `nomi-rt\n` and exits 0), `crates/xtask` skeleton,
`crates/nomi/tests/archive.rs`, `docs/diagnostics.md`, empty `lib/`. Acceptance: `nomi
--help` prints the synopsis; a test links the skeleton archive per §4.11 and the output
prints `nomi-rt` and exits 0; the archive test passes; clippy passes.

**CP2 — Runtime boot and fibers.** `nomi-rt/src/{start,tcb,stack,io,crash}.rs`. Tests: the
CP2 assembly programs of §7.

**CP3 — Allocator, RC and checking.** `nomi-rt/src/{shims,heap,check,stats,globals}.rs`.
Tests: unit tests and the CP3 assembly programs of §7. Acceptance also: a minimal
assembly `w_main` printing once links to ≤ 16 KiB stripped (§10).

**CP4 — Tracer bullet.** Minimal versions of every compiler stage for
`main = Stdio.print "hi\n"`: module header, imports, string literals (small layout),
application, qualified names, `Task` with `TaskOut` as `TaskPair`, the templates
`Task.succeed` and `Task.andThen`, closures with captures (closure `info` objects, the
`entry` call), top-level globals with `w_init` and `rt_immortalize_globals`, the
`Stdio.print` kernel, `w_main`, the kernel call sequence, stack checks, link;
`lib/nomi/platform/1.0.0/{elm.json,src/Stdio.elm}`,
`lib/nomi/core/1.0.0/{elm.json,src/Basics.elm,src/String.elm,src/Task.elm}` (the CP4
`elm.json` of §4.13), the harness `tests/programs.rs` and the program `hi`. Acceptance:
`hi` passes the four configurations and the stack-move pass.

**CP5 — Integers, functions and control.** Literals, arithmetic intrinsics, `negate`,
comparison, `if`, `let`, operator fixities and `infix`, recursion, the calling convention
with stack arguments and tail calls, polls, `rt_crash`, `&&`/`||`; `String.fromInt` and
`++` on small strings; lambdas, partial application, `apply_k` and indirect calls
(including indirect tail calls); programs `fib`, `divzero`, `minint`, `deep`, `deep7`,
`args`, `tailcalls`, `operators`, and `stackmove` with only its `count` line (its list
line is added in CP7). Acceptance: those programs pass.

**CP6 — Custom types and case.** Type declarations, constructors, case with decision
trees, argument-pattern checks, layouts and dispatch for `Imm`, `Tuple`, `Sum`, RC with
`imm_guard` for `Sum`, drop specialization; program `tree`. Acceptance: `tree` passes;
layout unit tests pass.

**CP7 — Lists, RC and reuse.** The list type, `[]`/`::`, `List.elm`, `Maybe.elm`, the full
`nomi/core` `elm.json` and prelude of §4.13, borrow and reuse for every layout, the reuse
report; programs `lists`, `bigdrop`, `layouts`, `sums`, `crossreuse`, and the full
`stackmove`. Acceptance: those pass; `--report reuse` on `tree` and `lists` shows hits in
`mapTree` and `List.map`; the `crossreuse` check of §7 holds.

**CP8 — Task.** The remaining Task templates (`fail`, `map`, `mapError`, `onError`) and
the `andThen` allocation check; programs `andthen/*`, `tasks` and `hello`. Acceptance:
those pass; the `andthen` check passes.

**CP9 — Strings and equality.** The large `String` layout, `k_String_eq`,
`k_String_compare`, `layout.rs`, `tests/layout.rs`, generated equality and comparison;
programs `strings`, `equality`. Acceptance: those pass; `tests/layout.rs` passes.

**CP10 — Completion.** Every diagnostic code with its snapshot; `docs/semantics.md`
(first version: `Int`, division, `String`, `Task`, program entry and exit, crashes,
stdout errors); `docs/grammar.md` complete for the subset; the I10 test; `bench/slice/*`,
`cargo xtask scoreboard`, `cargo xtask size`, `docs/scoreboard.md`. Acceptance (A§14 M1):
every program passes in all configurations and the stack-move pass; zero leaks under
`--rc-check --stack-initial 256`; `--stats` nonzero; reuse hits as in §7; `deep7` passes;
I10 and jobs equality hold; the kernel-use type test and the no-panic test pass;
`cargo xtask size` passes; the first scoreboard rows are written.

## 12. Review history

### Round 1
Five critics. Changes: tracer-bullet checkpoints (M1-D9); `TaskOut` as a `TaskPair` at
every opt level; `Kind`/`TypeLayout` for multi-constructor types; atom and `Rhs` encodings
within the POC's size asserts; `TyNode` 16 bytes without the `eq` flag; allocation sites
through every IR; LIR `Alloc` with an immediate-pool header, `Load8`, `imm_guard`,
explicit child decrements; the case dispatch rule; pseudo-instructions expanded after
register allocation with declared clobbers; the stack-argument convention with callee pop
and the tail-call copy order for m < k, m = k, m > k; F's terms; naive scratch
`rbx`/`r12`; checked builds through hooks, `heap::alloc`/`free`, site ids before kernel
calls, poisoned rc, size-preserving quarantine, `live_by_site`; mid-size reuse; large
regions; `mmap` failures; `string too large`; crash flush within 100 ms; stdout `EPIPE` →
141; stack-move on every check with fresh regions; link via same-directory rename; test
harness paths, deadlines and flags; every test program specified; diagnostics codes; CLI
defaults; `elm.json` files and placeholder declarations verbatim; signatures for every
stage; `Span` with file and `Diagnostic` per A§7.2; verified the clippy interaction of D24
and amended the architecture (D24, `lower_flat`, verifier placement, IRC rewrite in M4,
closure info tag, trimming deferred). Rejected: the reuse report's `borrowed` reason
(reuse runs before borrow inference).

### Round 2
Five critics. Changes: `TyNode` stays 12 bytes; global dense `TypeId` (fixed ids 0–9) and
`CtorId`, removing `ctor_refs`/`ctor_consts`; canonical `CoreTy`; CSR built by compaction
after inference; `TextRange` spans with `line_starts` (M1-D17); short-circuit `&&`/`||`;
kernel aliases and eta expansion; `idiv` guards (`minInt // -1`, never folded with a zero
divisor); optimizer passes treat the new forms as opaque except the simplifier and the RC
passes (M1-D14), with non-speculative field loads; free of objects over 64 words through
`rt_free` only; static closures with rc 0; opt 0 keeps the POC's path with kernel
ownership modes; sites inline in Flat (M1-D16) and per-site merge rules for the reuse
report; indirect tail calls; `IncSeq` with n and saturation, full `DecSeq`; geometric
mid-size classes (M1-D15); checked large objects and quarantine entries; reuse keeps the
original site; poison `0xDEAD_DEAD_DEAD_DEAC`; stack moves only up to 64 KiB used; crash
flush without changing file flags; flush before exit hooks; `--stack-initial` bounds;
`nomi run` signal statuses; `NOMI_AS`/`NOMI_LD` for toolchain diagnostics; E0702 dropped,
E0703 added; lib module headers; new programs `minint`, `equality`, `tasks`, `operators`,
`layouts`, `stackmove`, a wholesale `bigdrop`, exact `tailcalls` and `divzero`; shim and
`rt_free`-chain runtime tests; repository-root paths; I10 with `NOMI_JOBS`; the hash seed
moves to M4. The license stays open (`docs/plan-questions.md`).

### Round 3
Five critics. Changes: checkpoint order so every checkpoint's programs use only what it
builds (closures, globals and `Task.andThen` in CP4; strings, lambdas and `apply_k` in
CP5; `Sum` RC in CP6; `Maybe` and the full prelude in CP7); fixed `CtorId`s 0–9 with
placeholder declarations (M1-D19); Elm's shadowing rule as the oracle shows it (imported
names may be shadowed); labelled primary span (M1-D23); selective port (M1-D18) with
dense-id maps as `Vec`s, `ast/dump.rs` ported, `include_str!` of `runtime.s` and
`extra_args` removed on both sides, `Load.field` dropped; 12-byte `CoreTy` with `ListId`
slices; sites for `App`, partial applications, task closures and template lambdas,
`NO_SITE`; reuse reasons defined, kernel-only sites omitted; `DecSeq` frees through
`rt_free`, explicit children only from drop specialization; checked `FreeSeq` passes its
size; `StackCheck` resumes after the check; the definition of a checked function;
`apply_k` (M1-D20); the checked large threshold; the mid-size class formula; stats
counting points; one report writer (M1-D22); string capacities; stdout flushing for big
pieces; lexer reserved runs, `?`, negation of the next atom, invalid `\u` values;
argument-pattern exhaustiveness; the full diagnostics template table with E0111, E0112,
E0903; the crash flush never recurses; no panicking operations in `nomi-rt` (M1-D21,
measured at about 8 KB); `layout.rs` API and test procedure; kernel-use type test; runtime
assembly test names and expected-file format; programs `sums`, `crossreuse`, extended
`tasks` and `args`, exact `andthen` sources and the shared definitions; `RLIMIT_AS`
4 GiB; RSS columns on the scoreboard; jobs-equality test; `--symbols`; `_start` keeps the
initial `rsp`; static objects aligned; `()` in the equality row; package `elm-version`
ranges unchecked; the decision log sorted. Rejected: none.
