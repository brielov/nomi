# The solver ratchet

What upstream's dependency solver (`Deps/Solver.hs`) answers, scenario by
scenario, for each Compat. nomi must select what upstream selects, not
just some valid answer.

- `scenarios/<name>.json`: written by hand.
- `generated.json`: small random registries from a fixed seed, written by
  `cargo xtask oracle solver`; one scenario a line.
- `universes/`: registries that scenarios build on. `elm.json` is the
  published metadata of 99 real packages; `core.json` is `elm/core` and
  `elm/json` alone.
- `oracle.txt`: upstream's answers, written by `cargo xtask oracle solver`.
- `passing.txt`: the cases nomi answers the same way.

## Scenarios

```json
{
  "universe": "core",
  "registry": { "fix/a": ["2.0.0", "1.0.0"] },
  "releases": {
    "fix/a@2.0.0": { "elm": "0.19.0 <= v < 0.20.0", "deps": { "elm/core": "1.0.0 <= v < 2.0.0" } },
    "fix/a@1.0.0": { "elm": "0.19.0 <= v < 0.20.0", "src": false }
  },
  "project": { "type": "package", "elm": "0.19.0 <= v < 0.20.0", "deps": { "elm/core": "1.0.0 <= v < 2.0.0" } },
  "add": "fix/a"
}
```

- `registry` lists each package's versions, newest first, over the
  universe's.
- `releases` holds the metadata upstream finds in its package cache.
  - A release the registry lists without metadata isn't in the cache.
  - `"src": false` is a release whose sources are missing.
  - `"broken": true` is one whose `elm.json` is unreadable.
- `project` is an `application` (`direct`, `indirect`, `test-direct`,
  `test-indirect`, and an `elm` version that defaults to the Compat's own)
  or a `package` (`elm`, `deps`, `test-deps`).
  - An application must have `elm/core` among its direct dependencies and
    `elm/json` among its direct or indirect ones.
  - A package must depend on `elm/core`.
  - Upstream refuses the project otherwise.
- `add` is a package to add, as `install` does. Without it, the project's
  dependencies are checked, as `make` does.

## Answers

The solver runs offline, over the package cache alone. The first line of
each answer is one of these:

- `ok`, followed by:
  - for a check, the solution's releases, `<name> <version>` in name
    order;
  - for an application's install, the new dependencies:
    `direct|indirect|test-direct|test-indirect <name> <version>`;
  - for a package's install, the solution.
- `no-offline-solution`: no releases satisfy the constraints.
- `error`: a release's metadata is unreadable.
- `bad-outline`: the project lacks `elm/core` or `elm/json`.
- `bad-elm`: the project is for another elm.
- `hand-edited`: the dependencies contradict each other, or an
  application doesn't list exactly its solution.
- `already-installed`, `promote-indirect`, `promote-test`,
  `unknown-package`: `install` doesn't solve.

`oracles/solver/README.md` describes the oracle itself.
