import assert from "node:assert/strict";
import { test } from "node:test";

import { manual } from "../fixtures/driver.js";
import { TestClock } from "../src/clock.js";
import {
  all,
  andThen,
  die,
  ensuring,
  Exit,
  fail,
  interrupt,
  map,
  now,
  race,
  sleep,
  succeed,
  sync,
  yieldNow,
} from "../src/fiber.js";

/** A manual runtime on a test clock, with `log(line)` noting lines with the time. */
function timed() {
  const clock = new TestClock();
  const driven = manual({ clock });
  const lines = [];
  const log = (line) => sync(() => lines.push(`${clock.now()} ${line}`));
  return { ...driven, lines, log };
}

/** Sleeps `ms`, then ends with `task`, noting its finalizer as `name`. */
const after = (ms, task, name, log) => ensuring(andThen(sleep(ms), () => task), log(`${name} finalized`));

test("all succeeds with the values in order, whatever order they end in", () => {
  const { runtime, flush, lines, log } = timed();
  const fiber = runtime.fork(
    all([after(30, succeed(1), "a", log), after(10, succeed(2), "b", log), after(20, succeed(3), "c", log)]),
  );
  flush();
  assert.deepEqual(fiber.exit, Exit.success([1, 2, 3]));
  assert.deepEqual(lines, ["10 b finalized", "20 c finalized", "30 a finalized"]);
  assert.deepEqual(runtime.stats, { started: 4, finished: 4 });
});

test("all of nothing succeeds with nothing", () => {
  const { runtime, flush } = manual();
  const fiber = runtime.fork(all([]));
  flush();
  assert.deepEqual(fiber.exit, Exit.success([]));
});

test("all's first failure or defect is its own, once the rest are interrupted and finalized", () => {
  for (const [end, exit] of [
    [fail("x"), Exit.fail("x")],
    [die("d"), Exit.defect("d")],
  ]) {
    const { runtime, flush, reports, lines, log } = timed();
    const slow = after(3_600_000, succeed(1), "slow", log);
    const fiber = runtime.fork(andThen(all([slow, after(10, end, "fast", log)]), () => succeed("unreachable")));
    runtime.await(fiber);
    flush();
    assert.deepEqual(fiber.exit, exit);
    assert.deepEqual(lines, ["10 fast finalized", "10 slow finalized"]);
    assert.deepEqual(reports, []);
  }
});

test("race ends as the first to end does, and the loser is interrupted and finalized first", () => {
  const { runtime, flush, lines, log } = timed();
  const won = runtime.fork(
    andThen(race([after(50, succeed("slow"), "slow", log), after(10, succeed("fast"), "fast", log)]), (v) =>
      map(log(v), () => v),
    ),
  );
  const failed = runtime.fork(race([after(5, fail("x"), "failing", log), after(50, succeed("late"), "late", log)]));
  flush();
  assert.deepEqual(won.exit, Exit.success("fast"));
  assert.deepEqual(failed.exit, Exit.fail("x"));
  assert.deepEqual(lines, [
    "5 failing finalized",
    "5 late finalized",
    "10 fast finalized",
    "10 slow finalized",
    "10 fast",
  ]);
});

test("interrupting all or race interrupts the tasks it runs", () => {
  for (const combine of [all, race]) {
    const { runtime, flush, lines, log } = timed();
    const fiber = runtime.fork(combine([after(100, succeed(1), "a", log), after(200, succeed(2), "b", log)]));
    const stopper = runtime.fork(andThen(sleep(50), () => map(interrupt(fiber), (exit) => exit.$)));
    flush();
    assert.deepEqual(fiber.exit, Exit.interrupt());
    assert.deepEqual(stopper.exit, Exit.success("interrupt"));
    assert.deepEqual(lines, ["50 a finalized", "50 b finalized"]);
    assert.deepEqual(runtime.stats, { started: 4, finished: 4 });
  }
});

test("a task ending at once still ends all and race", () => {
  const { runtime, flush } = manual();
  const both = runtime.fork(all([succeed(1), andThen(yieldNow, () => succeed(2))]));
  const first = runtime.fork(race([succeed("now"), andThen(yieldNow, () => succeed("later"))]));
  const at = runtime.fork(all([now, now]));
  flush();
  assert.deepEqual(both.exit, Exit.success([1, 2]));
  assert.deepEqual(first.exit, Exit.success("now"));
  assert.equal(at.exit.$, "success");
});
