import assert from "node:assert/strict";
import { test } from "node:test";

import { check, rng as rngFor } from "../src/check.js";
import {
  andThen,
  async,
  awaitFiber,
  die,
  Exit,
  fail,
  fork,
  join,
  map,
  onError,
  Runtime,
  succeed,
  sync,
  yieldNow,
} from "../src/fiber.js";

// ---------------------------------------------------------------------------
// Model: random programs, run by the runtime under random schedules and
// compared with a direct evaluation. Fibers share no state, so a program's
// exit cannot depend on the schedule.

class Boom extends Error {}

function program(r, depth) {
  const leaf = depth <= 0;
  // Leaves are rarer above the bottom, so most programs have some shape.
  const l = leaf ? 1 : 0.25;
  const kind = r.weighted([
    [4 * l, "succeed"],
    [2 * l, "fail"],
    [1 * l, "throw"],
    [2 * l, "sync"],
    [3 * l, "async"],
    [leaf ? 0 : 3, "then"],
    [leaf ? 0 : 2, "bind"],
    [leaf ? 0 : 2, "catch"],
    [leaf ? 0 : 3, "par"],
    [leaf ? 0 : 1, "spawn"],
    [leaf ? 0 : 1, "yield"],
    [leaf ? 0 : 1, "notATask"],
  ]);
  const sub = () => program(r, depth - 1);
  switch (kind) {
    case "succeed":
    case "fail":
    case "throw":
    case "sync":
      return { kind, v: r.int(0, 9) };
    case "async":
      return { kind, v: r.int(0, 9), ok: r.bool(0.7), sync: r.bool(0.2) };
    case "then":
    case "catch":
      return { kind, a: sub(), add: r.int(1, 9) };
    case "bind":
      return { kind, a: sub(), b: sub() };
    case "par": {
      const children = r.array(sub, 1, 4);
      const order = children.map((_, i) => i);
      for (let i = order.length - 1; i > 0; i--) {
        const j = r.int(0, i);
        [order[i], order[j]] = [order[j], order[i]];
      }
      return { kind, children, order, mode: r.pick(["join", "await"]) };
    }
    case "spawn":
      return { kind, child: sub(), a: sub() };
    case "yield":
    case "notATask":
      return { kind, a: sub() };
  }
}

/** Builds the task for `p`; `pending` collects async operations for the driver. */
function build(p, pending) {
  switch (p.kind) {
    case "succeed":
      return succeed(p.v);
    case "fail":
      return fail(p.v);
    case "throw":
      return sync(() => {
        throw new Boom(String(p.v));
      });
    case "sync":
      return sync(() => p.v);
    case "async":
      return async((resume) => {
        const next = p.ok ? succeed(p.v) : fail(p.v);
        if (p.sync) resume(next);
        else pending.push(() => resume(next));
      });
    case "then":
      return map(build(p.a, pending), (x) => x + p.add);
    case "bind":
      return andThen(build(p.a, pending), () => build(p.b, pending));
    case "catch":
      return onError(build(p.a, pending), (e) => succeed(e + p.add));
    case "par": {
      const forkAll = p.children.reduceRight(
        (rest, child) => (fibers) => andThen(fork(build(child, pending)), (f) => rest([...fibers, f])),
        (fibers) => p.order.reduceRight((rest, i) => (sum) => wait(fibers[i], p.mode, sum, rest), succeed)(0),
      );
      return forkAll([]);
    }
    case "spawn":
      return andThen(fork(build(p.child, pending)), () => build(p.a, pending));
    case "yield":
      return andThen(yieldNow, () => build(p.a, pending));
    case "notATask":
      return andThen(build(p.a, pending), () => 42);
  }
}

function wait(fiber, mode, sum, rest) {
  return mode === "join"
    ? andThen(join(fiber), (x) => rest(sum + x))
    : andThen(awaitFiber(fiber), (exit) => rest(sum + exitCode(exit)));
}

function exitCode(exit) {
  switch (exit.$) {
    case "success":
      return exit.value;
    case "fail":
      return 1000 + exit.error;
    case "defect":
      return 2000;
    case "interrupt":
      return 3000;
  }
}

/**
 * The exit `p` must end with. `fibers` receives the model exit of every
 * fiber `p` starts and whether anyone ever waits for it.
 */
function model(p, fibers) {
  switch (p.kind) {
    case "succeed":
    case "sync":
      return Exit.success(p.v);
    case "fail":
      return Exit.fail(p.v);
    case "throw":
      return Exit.defect(`boom ${p.v}`);
    case "async":
      return p.ok ? Exit.success(p.v) : Exit.fail(p.v);
    case "then": {
      const x = model(p.a, fibers);
      return x.$ === "success" ? Exit.success(x.value + p.add) : x;
    }
    case "bind": {
      const x = model(p.a, fibers);
      return x.$ === "success" ? model(p.b, fibers) : x;
    }
    case "catch": {
      const x = model(p.a, fibers);
      return x.$ === "fail" ? Exit.success(x.error + p.add) : x;
    }
    case "par": {
      const children = p.children.map((child) => {
        const entry = { exit: model(child, fibers), observed: false };
        fibers.push(entry);
        return entry;
      });
      let sum = 0;
      for (const i of p.order) {
        const child = children[i];
        child.observed = true;
        if (p.mode === "await") {
          sum += exitCode(child.exit);
        } else if (child.exit.$ === "success") {
          sum += child.exit.value;
        } else {
          return child.exit;
        }
      }
      return Exit.success(sum);
    }
    case "spawn":
      fibers.push({ exit: model(p.child, fibers), observed: false });
      return model(p.a, fibers);
    case "yield":
      return model(p.a, fibers);
    case "notATask": {
      const x = model(p.a, fibers);
      return x.$ === "success" ? Exit.defect("TypeError") : x;
    }
  }
}

function normalize(exit) {
  if (exit.$ !== "defect" || typeof exit.defect === "string") return exit;
  const d = exit.defect;
  return Exit.defect(d instanceof Boom ? `boom ${d.message}` : d.name);
}

/** Runs `p` to completion, interleaving runtime slices and async completions at random. */
function runRandomly(p, r) {
  const slices = [];
  const pending = [];
  const reports = [];
  const runtime = new Runtime({
    schedule: (fn) => slices.push(fn),
    yieldEvery: r.int(1, 6),
    budget: r.int(1, 40),
    reportDefect: (defect, fiber) => reports.push(fiber.id),
  });
  const root = runtime.fork(build(p, pending));
  runtime.await(root);
  for (let guard = 0; slices.length > 0 || pending.length > 0; guard++) {
    assert.ok(guard < 1e6, "the program did not finish");
    if (slices.length > 0 && (pending.length === 0 || r.bool(0.7))) slices.shift()();
    else pending.splice(r.int(0, pending.length - 1), 1)[0]();
  }
  return { runtime, root, reports };
}

test("random programs end as the model says, under any schedule", async () => {
  await check(
    "fiber exits",
    (r) => ({ program: program(r, 5), schedule: r.next() }),
    ({ program: p, schedule }) => {
      const { runtime, root, reports } = runRandomly(p, rngFor(schedule));
      const fibers = [];
      assert.deepEqual(normalize(root.exit), normalize(model(p, fibers)));
      // Finished fibers leave every table.
      assert.equal(runtime.fibers.size, 0);
      assert.equal(runtime.stats.started, fibers.length + 1);
      assert.equal(runtime.stats.finished, runtime.stats.started);
      // Each defect is reported at most once, and never lost: a defect
      // nobody waits for is always reported.
      assert.equal(new Set(reports).size, reports.length);
      const defects = fibers.filter((f) => f.exit.$ === "defect");
      const unobserved = defects.filter((f) => !f.observed);
      assert.ok(reports.length >= unobserved.length, `${reports.length} reports, ${unobserved.length} unobserved defects`);
      assert.ok(reports.length <= defects.length);
    },
  );
});

// ---------------------------------------------------------------------------
// Unit tests

/** A runtime whose slices run when `flush` is called. */
function manual(options = {}) {
  const slices = [];
  const reports = [];
  const runtime = new Runtime({
    schedule: (fn) => slices.push(fn),
    reportDefect: (defect) => reports.push(defect),
    ...options,
  });
  const flush = () => {
    while (slices.length > 0) slices.shift()();
  };
  return { runtime, flush, reports };
}

test("a JS exception becomes a defect of its fiber", () => {
  const { runtime, flush, reports } = manual();
  const boom = new Error("boom");
  const fiber = runtime.fork(
    sync(() => {
      throw boom;
    }),
  );
  flush();
  assert.deepEqual(fiber.exit, Exit.defect(boom));
  assert.deepEqual(reports, [boom]);
});

test("a defect someone waits for is theirs, not reported", async () => {
  const { runtime, flush, reports } = manual();
  const exit = runtime.run(die("broken"));
  flush();
  assert.deepEqual(await exit, Exit.defect("broken"));
  assert.deepEqual(reports, []);
});

test("onError handles failures but not defects", () => {
  const { runtime, flush } = manual();
  const caught = runtime.fork(onError(fail(1), (e) => succeed(e + 1)));
  const uncaught = runtime.fork(onError(die("d"), () => succeed("handled")));
  flush();
  assert.deepEqual(caught.exit, Exit.success(2));
  assert.deepEqual(uncaught.exit, Exit.defect("d"));
});

test("a continuation returning a non-task is a defect", () => {
  const { runtime, flush } = manual();
  const fiber = runtime.fork(andThen(succeed(1), () => 2));
  flush();
  assert.equal(fiber.exit.$, "defect");
  assert.match(fiber.exit.defect.message, /expected a task, got number/);
});

test("resume takes effect once", () => {
  const { runtime, flush } = manual();
  let resume;
  const fiber = runtime.fork(async((r) => (resume = r)));
  flush();
  resume(succeed("first"));
  resume(succeed("second"));
  flush();
  resume(succeed("third"));
  flush();
  assert.deepEqual(fiber.exit, Exit.success("first"));
});

test("a throwing register is a defect unless it already resumed", () => {
  const { runtime, flush, reports } = manual();
  const before = runtime.fork(
    async(() => {
      throw new Error("before");
    }),
  );
  const after = runtime.fork(
    async((resume) => {
      resume(succeed("ok"));
      throw new Error("after");
    }),
  );
  runtime.await(before);
  flush();
  assert.equal(before.exit.defect.message, "before");
  assert.deepEqual(after.exit, Exit.success("ok"));
  assert.deepEqual(
    reports.map((e) => e.message),
    ["after"],
  );
});

test("a joiner ends as the joined fiber did", () => {
  const { runtime, flush } = manual();
  const fiber = runtime.fork(andThen(fork(fail("no")), join));
  flush();
  assert.deepEqual(fiber.exit, Exit.fail("no"));
});

test("long chains run in constant stack", () => {
  const { runtime, flush } = manual();
  let leftNested = succeed(0);
  for (let i = 0; i < 100_000; i++) leftNested = map(leftNested, (x) => x + 1);
  const loop = (i) => (i === 1_000_000 ? succeed(i) : andThen(succeed(i + 1), loop));
  const a = runtime.fork(leftNested);
  const b = runtime.fork(loop(0));
  flush();
  assert.deepEqual(a.exit, Exit.success(100_000));
  assert.deepEqual(b.exit, Exit.success(1_000_000));
});

test("a busy fiber cannot starve another", () => {
  const { runtime, flush } = manual({ yieldEvery: 16 });
  let stop = false;
  const spin = () => andThen(succeed(), () => (stop ? succeed("stopped") : spin()));
  const busy = runtime.fork(spin());
  const other = runtime.fork(
    sync(() => {
      stop = true;
      return "ran";
    }),
  );
  flush();
  assert.deepEqual(other.exit, Exit.success("ran"));
  assert.deepEqual(busy.exit, Exit.success("stopped"));
});

test("a busy runtime gives the event loop its turn", async () => {
  const runtime = new Runtime({ budget: 1000 });
  let stop = false;
  const spin = () => andThen(succeed(), () => (stop ? succeed("stopped") : spin()));
  const busy = runtime.run(spin());
  setTimeout(() => (stop = true), 5);
  assert.deepEqual(await busy, Exit.success("stopped"));
});
