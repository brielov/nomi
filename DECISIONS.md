# Decisions

One dated paragraph per decision that would be expensive to reverse. If a decision can be a test, it is a test instead.

**2026-10-07: test support is a dev-only crate.** Parity ratchets are checked from each crate's own tests, so the ratchet code must be shared between crates. It lives in `nomi-testkit`, which any crate may take as a dev-dependency and none as a normal one; `xtask ci` enforces both. The alternative, running every suite through `xtask`, would put compiler APIs behind a process boundary for no gain.

**2026-10-07: corpus claims are per Compat.** Upstream accepts an application only when its `elm-version` equals the compiler's version, and a package only when its range admits it; the pinned apps all say 0.19.1 and 14 corpus packages exclude 0.19.3. So exit criteria that say "every corpus package/app/project" (M5, 6.7, 7.4, 7.5, 8.10) now mean every one the Compat accepts, with the rejected ones kept as expected-failure cases whose diagnostics must match (plan 1.15a). Oracle sandboxes rewrite app `elm-version` to the Compat under test. The alternative, pinning only projects every Compat accepts, would drop the apps from 0.19.2 and 0.19.3 entirely.
