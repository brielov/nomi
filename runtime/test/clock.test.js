import assert from "node:assert/strict";
import { test } from "node:test";

import { check, rng as rngFor } from "../src/check.js";
import { RealClock, TestClock } from "../src/clock.js";
import { andThen, async, Exit, fork, interrupt, join, map, now, Runtime, sleep, succeed, sync } from "../src/fiber.js";

// ---------------------------------------------------------------------------
// Model: random programs of sleeps, host I/O and joined forks under a test
// clock. Virtual time is a pure function of the program: a step ends at the
// time the model says, whatever the schedule and however long I/O takes.

function program(r, depth) {
  const leaf = depth <= 0;
  const l = leaf ? 1 : 0.3;
  const kind = r.weighted([
    [3 * l, "sleep"],
    [2 * l, "io"],
    [1 * l, "now"],
    [leaf ? 0 : 3, "seq"],
    [leaf ? 0 : 2, "par"],
  ]);
  switch (kind) {
    case "sleep":
      return { kind, ms: r.pick([0, 0, 1, 5, 10, 25, 100]) };
    case "io":
    case "now":
      return { kind };
    case "seq":
    case "par":
      return { kind, children: r.array(() => program(r, depth - 1), 1, 4) };
  }
}

/** Sets `p.start` and `p.end`, the virtual times `p` starts and ends at. */
function model(p, start) {
  p.start = start;
  switch (p.kind) {
    case "sleep":
      p.end = start + p.ms;
      break;
    case "io":
    case "now":
      p.end = start;
      break;
    case "seq":
      p.end = p.children.reduce((t, child) => model(child, t), start);
      break;
    case "par":
      // Every child starts now; joining them all ends with the last.
      p.end = Math.max(start, ...p.children.map((child) => model(child, start)));
      break;
  }
  return p.end;
}

/** Builds `p`, recording the clock wherever the model makes a claim. */
function build(p, ctx) {
  const record = (what, expected) => sync(() => ctx.seen.push({ what, expected, actual: ctx.clock.now() }));
  let task;
  switch (p.kind) {
    case "sleep":
      task = sleep(p.ms);
      break;
    case "io":
      // Time must not move while the host is busy, however long it takes.
      task = andThen(
        async((resume) => void ctx.pending.push(() => resume(succeed()))),
        () => record("io end", p.start),
      );
      break;
    case "now":
      task = map(now, (t) => ctx.seen.push({ what: "now", expected: p.start, actual: t }));
      break;
    case "seq":
      task = p.children.reduceRight((rest, child) => () => andThen(build(child, ctx), rest), succeed)();
      break;
    case "par": {
      const forkAll = p.children.reduceRight(
        (rest, child) => (fibers) => andThen(fork(build(child, ctx)), (f) => rest([...fibers, f])),
        (fibers) => fibers.reduceRight((rest, f) => () => andThen(join(f), rest), succeed)(),
      );
      task = forkAll([]);
      break;
    }
  }
  return andThen(task, () => record(`${p.kind} end`, p.end));
}

test("virtual time depends on the program, not the schedule or I/O", async () => {
  await check(
    "virtual time",
    (r) => ({ program: program(r, 4), schedule: r.next() }),
    ({ program: p, schedule }) => {
      const r = rngFor(schedule);
      model(p, 0);
      const slices = [];
      const pending = [];
      const clock = new TestClock();
      const ctx = { clock, pending, seen: [] };
      const runtime = new Runtime({
        schedule: (fn) => slices.push(fn),
        yieldEvery: r.int(1, 6),
        budget: r.int(1, 40),
        clock,
      });
      const root = runtime.fork(build(p, ctx));
      for (let guard = 0; slices.length > 0 || pending.length > 0; guard++) {
        assert.ok(guard < 1e6, "the program did not finish");
        if (slices.length > 0 && (pending.length === 0 || r.bool(0.7))) slices.shift()();
        else pending.splice(r.int(0, pending.length - 1), 1)[0]();
      }
      assert.equal(root.exit.$, "success");
      assert.equal(clock.now(), p.end);
      assert.equal(clock.pending, 0);
      for (const { what, expected, actual } of ctx.seen) assert.equal(actual, expected, what);
    },
  );
});

// ---------------------------------------------------------------------------
// Unit tests

function manual(clock) {
  const slices = [];
  const runtime = new Runtime({ schedule: (fn) => slices.push(fn), clock });
  const flush = () => {
    while (slices.length > 0) slices.shift()();
  };
  return { runtime, flush };
}

test("a test clock jumps straight to the next timer", () => {
  const clock = new TestClock(1000);
  const { runtime, flush } = manual(clock);
  const fiber = runtime.fork(andThen(sleep(60_000), () => now));
  flush();
  assert.deepEqual(fiber.exit, Exit.success(61_000));
});

test("timers due at the same instant fire in the order they were set", () => {
  const clock = new TestClock();
  const { runtime, flush } = manual(clock);
  const order = [];
  for (const name of ["a", "b", "c"]) runtime.fork(andThen(sleep(5), () => sync(() => order.push(name))));
  runtime.fork(andThen(sleep(1), () => sync(() => order.push("first"))));
  flush();
  assert.deepEqual(order, ["first", "a", "b", "c"]);
});

test("a test clock stands still while I/O is pending", () => {
  const clock = new TestClock();
  const { runtime, flush } = manual(clock);
  let finishIo;
  const sleeper = runtime.fork(sleep(10));
  const io = runtime.fork(async((resume) => (finishIo = resume)));
  flush();
  assert.equal(clock.now(), 0);
  assert.equal(sleeper.exit, null);
  finishIo(succeed("read"));
  flush();
  assert.deepEqual(io.exit, Exit.success("read"));
  assert.deepEqual(sleeper.exit, Exit.success(undefined));
  assert.equal(clock.now(), 10);
});

test("interrupting a sleeper cancels its timer", () => {
  const clock = new TestClock();
  const { runtime, flush } = manual(clock);
  const sleeper = runtime.fork(sleep(1_000));
  const killer = runtime.fork(andThen(sleep(5), () => interrupt(sleeper)));
  flush();
  assert.deepEqual(sleeper.exit, Exit.interrupt());
  assert.deepEqual(killer.exit, Exit.success(Exit.interrupt()));
  assert.equal(clock.pending, 0);
  assert.equal(clock.now(), 5);
});

test("the real clock sleeps on host timers", async () => {
  const runtime = new Runtime({ clock: new RealClock() });
  const before = Date.now();
  const exit = await runtime.run(andThen(sleep(20), () => now));
  assert.equal(exit.$, "success");
  assert.ok(exit.value - before >= 19, `${exit.value - before} ms`);
});
