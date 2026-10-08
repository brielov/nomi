# The solver oracle

`Main.hs` runs upstream's own dependency solver (`Deps/Solver.hs`) the way
`make` and `install` run it, over a fixture registry and package cache.
`cargo xtask oracle solver` compiles it against each Compat's reference
sources, runs it on every scenario of `crates/registry/tests/solver/`, and
writes the answers to that suite's `oracle.txt`.

## Building

It needs the GHC on `PATH` (9.14.1 builds it) and upstream's Haskell
dependencies in the cabal store:

```sh
cd reference/compiler && cabal build --only-dependencies
```

xtask finds the store's package database for that GHC under
`~/.local/state/cabal/store`, or takes `NOMI_GHC_PACKAGE_DB`. It compiles
the oracle with `ghc --make` directly, not through cabal, so no
`elm.cabal` is edited, and caches each build under the development cache,
keyed by the GHC, the sources' commit, `Main.hs` and the shim.

The modules that moved between releases go through a `Compat` shim:

- `compat-0.19.2/` (`Stuff`) serves 0.19.1 and 0.19.2.
- `compat-0.19.3/` (`Root`) serves 0.19.3.

`Paths_elm`, which cabal would generate, is written per build. It sets
`V.compiler`, the release the oracle claims to be.

## 0.19.1

0.19.1's sources no longer compile: `Data/Utf8.hs` mixes `Word#` and
`Word8#`, which GHC 9.2 split, so GHC 9.8 fails the same way. Its oracle
is built from 0.19.2's sources with `Paths_elm` at 0.19.1 instead. Between
the two tags:

- `Deps/Solver.hs` changes only in how decoding threads through IO;
- `Elm/Outline.hs` and `Elm/Constraint.hs` change only in parser plumbing.

So the search, the constraint arithmetic and the checks against
`V.compiler` are 0.19.1's.
