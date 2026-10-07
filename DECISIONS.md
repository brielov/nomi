# Decisions

One dated paragraph per decision that would be expensive to reverse. If a decision can be a test, it is a test instead.

**2026-10-07: test support is a dev-only crate.** Parity ratchets are checked from each crate's own tests, so the ratchet code must be shared between crates. It lives in `nomi-testkit`, which any crate may take as a dev-dependency and none as a normal one; `xtask ci` enforces both. The alternative, running every suite through `xtask`, would put compiler APIs behind a process boundary for no gain.
