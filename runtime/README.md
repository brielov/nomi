# runtime

The fiber runtime behind nomi's Node platform (architecture §8, "Runtime"). Plain JS, no dependencies, tested with `node --test`.

- `src/fiber.js`: tasks, fibers, scopes, interruption and the run queue.
- `src/clock.js`: the real clock and the virtual test clock.
- `src/main.js`: running a program as a process.
- `src/check.js`: the seeded model checker the tests use.

`NOMI_SEEDS=10000 node --test` runs every property with 10,000 seeds. A failure prints the line that replays it.

## Where each rule is tested

| Rule (architecture §8) | Tests |
|---|---|
| Structured concurrency | `test/fiber.test.js`: the random-program property (a scope's fibers end before it does), "closing a scope interrupts its fibers…", "finalizers run last to first…", "a fiber's children end with it; a daemon outlives it" |
| Interruption | `test/fiber.test.js`: "acquire is uninterruptible…", the "Interruption" section, and the property (a release runs exactly when its acquire did; an uninterruptible region runs to its end) |
| Exits | `test/fiber.test.js`: the property (exits match the model; defects are reported exactly when nobody receives them) and the unit tests on defects |
| Fairness | `test/fiber.test.js`: "a busy fiber cannot starve another", "a busy runtime gives the event loop its turn" |
| Bookkeeping | `test/fiber.test.js`: the property (finished fibers leave every table), "a runtime at rest keeps no finished fiber's result alive"; `test/lifecycle.test.js`: "the runtime's metrics are complete when main resolves…" |
| Lifecycle | `test/lifecycle.test.js`, which runs `fixtures/program.js` in child processes |
| Test clock | `test/clock.test.js` |
