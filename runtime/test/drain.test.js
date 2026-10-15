import assert from "node:assert/strict";
import { test } from "node:test";

import { manual } from "../fixtures/driver.js";
import { TestClock } from "../src/clock.js";
import {
  acquireRelease,
  addFinalizer,
  andThen,
  draining,
  ensuring,
  Exit,
  interrupt,
  scoped,
  sleep,
  succeed,
  sync,
} from "../src/fiber.js";

/**
 * A server on a test clock: a draining scope of `ms` whose accept loop, in
 * a nested scope that notes when it closes, takes the next of `requests`
 * every 10 ms and spawns it, keeping its fiber in `fibers`. `requests[i]`
 * is called with `spawn`. Something interrupts the server at 25 ms, so two
 * requests are accepted.
 */
function serve(ms, requests) {
  const clock = new TestClock();
  const { runtime, flush, reports } = manual({ clock });
  const lines = [];
  const log = (line) => sync(() => lines.push(`${clock.now()} ${line}`));
  const fibers = [];
  const server = runtime.fork(
    draining(ms, (spawn) => {
      const accept = (i) =>
        andThen(sleep(10), () =>
          andThen(spawn(requests[i](spawn)), (fiber) => (fibers.push(fiber), accept(i + 1))),
        );
      const listening = acquireRelease(log("listening"), () => log("closed"));
      return andThen(addFinalizer(() => log("server finalized")), () => scoped(andThen(listening, () => accept(0))));
    }),
  );
  runtime.fork(andThen(sleep(25), () => interrupt(server)));
  return { runtime, flush, reports, clock, lines, log, fibers, server };
}

test("a draining scope starts nothing once it closes", () => {
  let late = null;
  const s = serve(100, [
    () => sleep(50),
    (spawn) =>
      andThen(sleep(20), () =>
        andThen(spawn(sync(() => assert.fail("a late request ran"))), (fiber) => ((late = fiber), sleep(0))),
      ),
    () => assert.fail("the accept loop ran on"),
  ]);
  s.flush();
  assert.deepEqual(s.lines, ["0 listening", "25 closed", "60 server finalized"]);
  assert.equal(s.fibers.length, 2);
  assert.deepEqual(late.exit, Exit.interrupt());
  assert.deepEqual(s.reports, []);
  assert.deepEqual(s.runtime.stats, { started: 5, finished: 5 });
});

test("in-flight fibers that end before the deadline keep their results, and it ends with them", () => {
  const s = serve(100, [() => andThen(sleep(30), () => succeed("a")), () => andThen(sleep(70), () => succeed("b"))]);
  s.flush();
  assert.deepEqual(
    s.fibers.map((f) => f.exit),
    [Exit.success("a"), Exit.success("b")],
  );
  assert.deepEqual(s.lines, ["0 listening", "25 closed", "90 server finalized"]);
  assert.deepEqual(s.server.exit, Exit.interrupt());
  // The deadline's timer went with the drain, so time stopped at its end.
  assert.equal(s.clock.now(), 90);
  assert.equal(s.clock.pending, 0);
});

test("past the deadline the rest are interrupted and finalized, then the scope's finalizers run", () => {
  const s = serve(100, [
    () => andThen(sleep(30), () => succeed("quick")),
    () => ensuring(andThen(sleep(3_600_000), () => succeed("slow")), s.log("slow finalized")),
  ]);
  s.flush();
  assert.deepEqual(
    s.fibers.map((f) => f.exit),
    [Exit.success("quick"), Exit.interrupt()],
  );
  assert.deepEqual(s.lines, ["0 listening", "25 closed", "125 slow finalized", "125 server finalized"]);
  assert.deepEqual(s.runtime.stats, { started: 4, finished: 4 });
});

test("a draining scope that ends on its own drains too", () => {
  const clock = new TestClock();
  const { runtime, flush } = manual({ clock });
  const fiber = runtime.fork(draining(100, (spawn) => andThen(spawn(sleep(40)), () => succeed("done"))));
  flush();
  assert.deepEqual(fiber.exit, Exit.success("done"));
  assert.equal(clock.now(), 40);
});
