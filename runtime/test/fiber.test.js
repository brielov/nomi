import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { manual, shuffled } from "../fixtures/driver.js";
import { check, rng as rngFor } from "../src/check.js";
import {
  acquireRelease,
  addFinalizer,
  andThen,
  async,
  awaitFiber,
  die,
  ensuring,
  Exit,
  fail,
  fork,
  forkDaemon,
  fromPromise,
  Interrupted,
  interrupt,
  interruptFork,
  join,
  map,
  onError,
  Runtime,
  scoped,
  succeed,
  sync,
  uninterruptible,
  withRuntime,
  yieldNow,
} from "../src/fiber.js";

// ---------------------------------------------------------------------------
// Model: random programs, run by the runtime under random schedules and
// compared with a direct evaluation. Fibers share no state, so a program's
// exit cannot depend on the schedule. What a fiber does after it may have
// been interrupted can, so the model marks those fibers uncertain and the
// property only bounds them.

class Boom extends Error {}

/** A task left out of each constructor that takes one. */
const MISSING = {
  andThen: andThen(undefined, succeed),
  onError: onError(null, fail),
  uninterruptible: uninterruptible(null),
  scoped: scoped(undefined),
};

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
    [1 * l, "missing"],
    [leaf ? 0 : 3, "then"],
    [leaf ? 0 : 2, "bind"],
    [leaf ? 0 : 2, "catch"],
    [leaf ? 0 : 3, "par"],
    [leaf ? 0 : 1, "spawn"],
    [leaf ? 0 : 1, "yield"],
    [leaf ? 0 : 1, "notATask"],
    [leaf ? 0 : 2, "ensure"],
    [leaf ? 0 : 1, "scope"],
    [leaf ? 0 : 1, "mask"],
    [leaf ? 0 : 1, "kill"],
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
    case "missing":
      return { kind, via: r.pick(Object.keys(MISSING)) };
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
    case "kill":
      return { kind, child: sub(), a: sub(), wait: r.bool(0.5) };
    case "ensure":
      return { kind, a: sub(), fin: sub() };
    case "yield":
    case "notATask":
    case "scope":
    case "mask":
      return { kind, a: sub() };
  }
}

/**
 * Builds the task for `p`. `ctx.pending` collects async operations for the
 * driver; `ctx.forks` the fibers forked under the innermost checked scope;
 * `ctx.log` what ran, for the properties to check.
 */
function build(p, ctx) {
  const pending = ctx.pending;
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
    case "missing":
      return MISSING[p.via];
    case "then":
      return map(build(p.a, ctx), (x) => x + p.add);
    case "bind":
      return andThen(build(p.a, ctx), () => build(p.b, ctx));
    case "catch":
      return onError(build(p.a, ctx), (e) => succeed(e + p.add));
    case "par": {
      const forkAll = p.children.reduceRight(
        (rest, child) => (fibers) =>
          andThen(fork(checked(child, ctx)), (f) => (ctx.forks.push(f), rest([...fibers, f]))),
        (fibers) => p.order.reduceRight((rest, i) => (sum) => wait(fibers[i], p.mode, sum, rest), succeed)(0),
      );
      return forkAll([]);
    }
    case "spawn":
      // A daemon belongs to no scope of ours.
      return andThen(forkDaemon(checked(p.child, ctx)), () => build(p.a, ctx));
    case "kill":
      return andThen(fork(checked(p.child, ctx)), (f) => {
        ctx.forks.push(f);
        return andThen(p.wait ? interrupt(f) : interruptFork(f), () => build(p.a, ctx));
      });
    case "yield":
      return andThen(yieldNow, () => build(p.a, ctx));
    case "notATask":
      return andThen(build(p.a, ctx), () => 42);
    case "ensure": {
      // Acquiring and registering the release are one step, so the release
      // runs exactly when the acquire did, however the fiber ends.
      const id = ctx.log.ensures.push({ acquired: 0, released: 0 }) - 1;
      const entry = ctx.log.ensures[id];
      return scoped(
        andThen(
          acquireRelease(
            sync(() => entry.acquired++),
            () => andThen(sync(() => entry.released++), () => build(p.fin, ctx)),
          ),
          () => build(p.a, ctx),
        ),
      );
    }
    case "scope":
      return scoped(checked(p.a, ctx));
    case "mask": {
      const id = ctx.log.masks.push({ started: 0, ended: 0, node: p }) - 1;
      const entry = ctx.log.masks[id];
      const end = () => entry.ended++;
      return uninterruptible(
        andThen(sync(() => entry.started++), () =>
          onError(
            map(build(p.a, ctx), (x) => (end(), x)),
            (e) => (end(), fail(e)),
          ),
        ),
      );
    }
  }
}

/** `p` in a scope, a fiber's own or a `scoped` one, whose fibers must all be done when it closes. */
function checked(p, ctx) {
  const forks = [];
  const outlived = () =>
    sync(() => {
      for (const f of forks) if (f.exit === null) ctx.log.outlived.push(f.id);
    });
  return andThen(addFinalizer(outlived), () => build(p, { ...ctx, forks }));
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
 * The exit `p` must end with. `entries` receives an entry for every fiber
 * `p` starts, with its model exit and whether anyone ever waits for it, and
 * one for every defect a finalizer adds to an exit that already had one,
 * which the runtime reports instead.
 */
function model(p, entries) {
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
    case "missing":
      return Exit.defect("TypeError");
    case "then": {
      const x = model(p.a, entries);
      return x.$ === "success" ? Exit.success(x.value + p.add) : x;
    }
    case "bind": {
      const x = model(p.a, entries);
      return x.$ === "success" ? model(p.b, entries) : x;
    }
    case "catch": {
      const x = model(p.a, entries);
      return x.$ === "fail" ? Exit.success(x.error + p.add) : x;
    }
    case "par": {
      const children = p.children.map((child) => {
        const below = [];
        const entry = fiberEntry(model(child, below));
        entries.push(entry, ...below);
        return { entry, below };
      });
      let sum = 0;
      for (const [k, i] of p.order.entries()) {
        const { entry } = children[i];
        entry.observed = true;
        if (p.mode === "await") {
          sum += exitCode(entry.exit);
        } else if (entry.exit.$ === "success") {
          sum += entry.exit.value;
        } else {
          // The rest are left running until their scope closes.
          for (const j of p.order.slice(k + 1)) uncertain(children[j].entry, children[j].below);
          return entry.exit;
        }
      }
      return Exit.success(sum);
    }
    case "spawn":
    case "kill": {
      const below = [];
      const entry = fiberEntry(model(p.child, below));
      if (p.kind === "kill") {
        // Only an interrupt that waits receives the exit.
        entry.observed = p.wait;
        uncertain(entry, below);
      }
      entries.push(entry, ...below);
      return model(p.a, entries);
    }
    case "yield":
    case "scope":
      return model(p.a, entries);
    case "mask": {
      const x = model(p.a, entries);
      p.aExit = x.$;
      return x;
    }
    case "notATask": {
      const x = model(p.a, entries);
      return x.$ === "success" ? Exit.defect("TypeError") : x;
    }
    case "ensure": {
      const x = model(p.a, entries);
      const f = model(p.fin, entries);
      if (f.$ !== "fail" && f.$ !== "defect") return x;
      const defect = f.$ === "fail" ? f.error : f.defect;
      if (x.$ !== "defect") return Exit.defect(defect);
      entries.push({ kind: "dropped", exit: Exit.defect(defect), observed: false, certain: true });
      return x;
    }
  }
}

function fiberEntry(exit) {
  return { kind: "fiber", exit, observed: false, certain: true };
}

/** The fiber may be interrupted at any point, and so may anything it starts. */
function uncertain(entry, below) {
  entry.certain = false;
  for (const e of below) e.certain = false;
}

function normalize(exit) {
  if (exit.$ !== "defect" || typeof exit.defect !== "object") return exit;
  const d = exit.defect;
  return Exit.defect(d instanceof Boom ? `boom ${d.message}` : d.name);
}

/**
 * Runs `p` to completion, interleaving runtime slices and async completions
 * at random, on a runtime that drains synchronously or not. A synchronous
 * one may finish the root before anyone waits for it: `early` says so.
 */
function runRandomly(p, r) {
  const { runtime, reports, pending, settle } = shuffled(r, { synchronous: r.bool(0.5) });
  const log = { ensures: [], masks: [], outlived: [] };
  const root = runtime.fork(checked(p, { pending, log }));
  const early = root.exit !== null;
  runtime.await(root);
  settle();
  return { runtime, root, early, reports, log };
}

const count = (xs, p) => xs.filter(p).length;

test("random programs end as the model says, under any schedule", async () => {
  await check(
    (r) => ({ program: program(r, 5), schedule: r.next() }),
    ({ program: p, schedule }) => {
      const entries = [];
      const expected = model(p, entries);
      const { runtime, root, early, reports, log } = runRandomly(p, rngFor(schedule));
      assert.deepEqual(normalize(root.exit), normalize(expected));

      // Finished fibers leave every table.
      assert.equal(runtime.root.fibers.size, 0);
      const fibers = entries.filter((e) => e.kind === "fiber");
      assert.ok(runtime.stats.started >= count(fibers, (e) => e.certain) + 1);
      assert.ok(runtime.stats.started <= fibers.length + 1);
      assert.equal(runtime.stats.finished, runtime.stats.started);

      // A defect is reported exactly when nobody receives it: never lost,
      // never twice.
      const unobserved = (e) => e.exit.$ === "defect" && !e.observed;
      const rootReported = early && root.exit.$ === "defect" ? 1 : 0;
      const atLeast = count(entries, (e) => e.certain && unobserved(e)) + rootReported;
      const atMost = count(entries, (e) => e.exit.$ === "defect") + rootReported;
      assert.ok(reports.length >= atLeast, `${reports.length} reports, at least ${atLeast} expected`);
      assert.ok(reports.length <= atMost, `${reports.length} reports, at most ${atMost} expected`);

      // A scope's fibers end before it does.
      assert.deepEqual(log.outlived, []);
      // A release runs exactly when its acquire did.
      for (const e of log.ensures) {
        assert.ok(e.acquired <= 1);
        assert.equal(e.released, e.acquired);
      }
      // An uninterruptible region that starts runs to its end.
      for (const m of log.masks) {
        assert.ok(m.started <= 1);
        if (m.node.aExit !== "defect") assert.equal(m.ended, m.started);
      }
    },
  );
});

// ---------------------------------------------------------------------------
// Unit tests

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

test("a continuation or resume handing back a non-task, an exit among them, is a defect", () => {
  const { runtime, flush } = manual();
  const results = [2, Exit.success(1), Exit.fail(1), Exit.defect(1), Exit.interrupt()];
  const continued = results.map((result) => runtime.fork(andThen(succeed(1), () => result)));
  const resumed = runtime.fork(async((resume) => resume(Exit.interrupt())));
  flush();
  assert.deepEqual(
    [...continued, resumed].map((f) => f.exit.defect.message),
    ["expected a task, got number", ...Array(5).fill("expected a task, got Object")],
  );
});

test("a task missing where one belongs is a defect of its fiber, and the runtime runs on", () => {
  const { runtime, flush, reports } = manual();
  const fibers = Object.values(MISSING).map((task) => runtime.fork(task));
  flush();
  const later = runtime.fork(succeed("ran"));
  flush();
  for (const f of fibers) assert.match(f.exit.defect.message, /^expected a task, got (null|undefined)$/);
  assert.equal(reports.length, fibers.length);
  assert.deepEqual(later.exit, Exit.success("ran"));
});

test("awaiting or interrupting a non-fiber is a defect naming the constructor", () => {
  const { runtime, flush } = manual();
  const awaited = runtime.fork(awaitFiber("f"));
  const interrupted = runtime.fork(interrupt(undefined));
  flush();
  assert.equal(awaited.exit.defect.message, "awaitFiber expects a fiber, got string");
  assert.equal(interrupted.exit.defect.message, "interrupt expects a fiber, got undefined");
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

test("withRuntime gives each fiber the runtime running it", () => {
  const one = manual();
  const other = manual();
  const asked = andThen(fork(withRuntime(succeed)), join);
  const a = one.runtime.fork(asked);
  const b = other.runtime.fork(asked);
  one.flush();
  other.flush();
  assert.equal(a.exit.value, one.runtime);
  assert.equal(b.exit.value, other.runtime);
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

test("a runtime at rest keeps no finished fiber's result alive", () => {
  // Collecting garbage on demand needs --expose-gc, so it runs in a child.
  const retention = fileURLToPath(new URL("../fixtures/retention.js", import.meta.url));
  const { stdout, stderr } = spawnSync(process.execPath, ["--expose-gc", retention], { encoding: "utf8" });
  assert.equal(stdout.trim(), "collected", stderr);
});

// ---------------------------------------------------------------------------
// Scopes and finalizers

/** An async task that never resumes on its own; `cancelled` counts cancels. */
function never(state = { cancelled: 0 }) {
  return async(() => () => state.cancelled++);
}

test("finalizers run last to first, with the scope's exit", () => {
  const { runtime, flush } = manual();
  const log = [];
  const note = (name) => (exit) => sync(() => log.push(`${name}:${exit.$}`));
  const fiber = runtime.fork(
    andThen(
      scoped(andThen(addFinalizer(note("a")), () => andThen(addFinalizer(note("b")), () => fail("x")))),
      () => succeed("unreachable"),
    ),
  );
  flush();
  assert.deepEqual(log, ["b:fail", "a:fail"]);
  assert.deepEqual(fiber.exit, Exit.fail("x"));
});

test("closing a scope interrupts its fibers and waits for them before finalizing", () => {
  const { runtime, flush } = manual();
  const log = [];
  const state = { cancelled: 0 };
  const child = ensuring(never(state), sync(() => log.push("child released")));
  let forked;
  const fiber = runtime.fork(
    scoped(
      andThen(addFinalizer(() => sync(() => log.push(`parent finalized, child ${forked.exit.$}`))), () =>
        andThen(fork(child), (f) => {
          forked = f;
          // Let the child start, so it is interrupted while it waits.
          return map(yieldNow, () => "done");
        }),
      ),
    ),
  );
  flush();
  assert.deepEqual(fiber.exit, Exit.success("done"));
  assert.deepEqual(log, ["child released", "parent finalized, child interrupt"]);
  assert.equal(state.cancelled, 1);
  assert.deepEqual(runtime.stats, { started: 2, finished: 2 });
});

test("a fiber's children end with it; a daemon outlives it", () => {
  const { runtime, flush } = manual();
  let child;
  let daemon;
  let resumeDaemon;
  const parent = runtime.fork(
    andThen(fork(never()), (c) => {
      child = c;
      return map(forkDaemon(async((resume) => (resumeDaemon = resume))), (d) => (daemon = d));
    }),
  );
  flush();
  assert.equal(parent.exit.$, "success");
  assert.deepEqual(child.exit, Exit.interrupt());
  assert.equal(daemon.exit, null);
  resumeDaemon(succeed("still here"));
  flush();
  assert.deepEqual(daemon.exit, Exit.success("still here"));
});

test("shutdown also ends the fibers its finalizers start", async () => {
  const { runtime, flush } = manual();
  const log = [];
  const daemon = ensuring(never(), sync(() => log.push("daemon released")));
  // The finalizer yields, so the daemon starts and registers its own.
  const release = andThen(forkDaemon(daemon), () => andThen(yieldNow, () => sync(() => log.push("released"))));
  runtime.fork(ensuring(never(), release));
  flush();
  let ended = false;
  const shutdown = runtime.shutdown().then(() => (ended = true));
  for (let i = 0; i < 5 && !ended; i++) {
    flush();
    await Promise.resolve();
  }
  await shutdown;
  assert.deepEqual(log, ["released", "daemon released"]);
  assert.equal(runtime.root.fibers.size, 0);
  assert.deepEqual(runtime.stats, { started: 2, finished: 2 });
});

test("acquire is uninterruptible and release runs exactly once", () => {
  const { runtime, flush } = manual();
  const log = [];
  let finishAcquire;
  const fiber = runtime.fork(
    scoped(
      andThen(
        acquireRelease(
          async((resume) => (finishAcquire = resume)),
          (resource, exit) => sync(() => log.push(`release ${resource} after ${exit.$}`)),
        ),
        () => never(),
      ),
    ),
  );
  flush();
  const interrupted = runtime.interrupt(fiber);
  flush();
  // Still acquiring: the interruption waits.
  assert.equal(fiber.exit, null);
  finishAcquire(succeed("handle"));
  flush();
  assert.deepEqual(fiber.exit, Exit.interrupt());
  assert.deepEqual(log, ["release handle after interrupt"]);
  return interrupted.then((exit) => assert.deepEqual(exit, Exit.interrupt()));
});

test("ensuring runs its finalizer however the task ends", () => {
  const { runtime, flush } = manual();
  const ends = [succeed(1), fail(2), die("3"), never()];
  const log = [];
  const fibers = ends.map((task, i) => runtime.fork(ensuring(task, sync(() => log.push(i)))));
  flush();
  runtime.interrupt(fibers[3]);
  flush();
  assert.deepEqual(log, [0, 1, 2, 3]);
  assert.deepEqual(
    fibers.map((f) => f.exit.$),
    ["success", "fail", "defect", "interrupt"],
  );
});

test("a failing finalizer is a defect; one after a defect is reported", () => {
  const { runtime, flush, reports } = manual();
  const failed = runtime.fork(ensuring(succeed("ok"), fail("finalizer")));
  const twice = runtime.fork(ensuring(die("first"), die("second")));
  runtime.await(twice);
  flush();
  assert.deepEqual(failed.exit, Exit.defect("finalizer"));
  assert.deepEqual(twice.exit, Exit.defect("first"));
  assert.deepEqual(reports, ["finalizer", "second"]);
});

test("interrupt waits for the fiber and succeeds with its exit", () => {
  const { runtime, flush } = manual();
  const log = [];
  const fiber = runtime.fork(
    andThen(fork(ensuring(never(), sync(() => log.push("released")))), (f) =>
      andThen(andThen(yieldNow, () => interrupt(f)), (exit) =>
        sync(() => (log.push(`interrupted: ${exit.$}`), exit.$)),
      ),
    ),
  );
  flush();
  assert.deepEqual(fiber.exit, Exit.success("interrupt"));
  assert.deepEqual(log, ["released", "interrupted: interrupt"]);
});

test("an interrupted waiter stops counting as waiting", () => {
  const { runtime, flush, reports } = manual();
  let resume;
  const target = runtime.fork(async((r) => (resume = r)));
  const waiter = runtime.fork(awaitFiber(target));
  flush();
  runtime.interrupt(waiter);
  flush();
  resume(die("nobody is waiting"));
  flush();
  assert.deepEqual(waiter.exit, Exit.interrupt());
  assert.deepEqual(reports, ["nobody is waiting"]);
});

test("an interruption during an uninterruptible region lands when it ends", () => {
  const { runtime, flush } = manual();
  const log = [];
  let resume;
  const fiber = runtime.fork(
    andThen(
      uninterruptible(map(async((r) => (resume = r)), (x) => log.push(`region ended with ${x}`))),
      () => sync(() => log.push("unreachable")),
    ),
  );
  flush();
  runtime.interrupt(fiber);
  flush();
  resume(succeed(1));
  flush();
  assert.deepEqual(log, ["region ended with 1"]);
  assert.deepEqual(fiber.exit, Exit.interrupt());
});

// ---------------------------------------------------------------------------
// Interruption

test("interruption lands at the next yield point, not mid-step", () => {
  const { runtime, flush } = manual({ yieldEvery: 1000 });
  const log = [];
  let self;
  const fiber = runtime.fork(
    andThen(
      sync(() => {
        // Requested from inside the fiber's own step: the step finishes.
        runtime.interrupt(self);
        log.push("same step");
      }),
      () =>
        andThen(sync(() => log.push("still the same step")), () =>
          andThen(yieldNow, () => sync(() => log.push("after the yield"))),
        ),
    ),
  );
  self = fiber;
  flush();
  assert.deepEqual(log, ["same step", "still the same step"]);
  assert.deepEqual(fiber.exit, Exit.interrupt());
});

test("a busy fiber is interrupted at a forced yield", () => {
  const { runtime, flush } = manual({ yieldEvery: 8, budget: 8 });
  let spins = 0;
  const spin = () => andThen(sync(() => spins++), spin);
  const busy = runtime.fork(spin());
  runtime.fork(sync(() => runtime.interrupt(busy)));
  flush();
  assert.deepEqual(busy.exit, Exit.interrupt());
  assert.ok(spins > 0 && spins < 16, `${spins} spins`);
});

test("a suspended fiber is interrupted at once and its operation cancelled", () => {
  const { runtime, flush } = manual();
  const state = { cancelled: 0 };
  let resume;
  const fiber = runtime.fork(async((r) => ((resume = r), () => state.cancelled++)));
  flush();
  runtime.interrupt(fiber);
  runtime.interrupt(fiber);
  flush();
  assert.deepEqual(fiber.exit, Exit.interrupt());
  assert.equal(state.cancelled, 1);
  // The host finishing late changes nothing.
  resume(succeed("late"));
  flush();
  assert.deepEqual(fiber.exit, Exit.interrupt());
});

test("an uninterruptible fiber keeps going and is interrupted after", () => {
  const { runtime, flush } = manual({ yieldEvery: 2 });
  const log = [];
  const fiber = runtime.fork(
    andThen(
      uninterruptible(
        andThen(yieldNow, () =>
          andThen(yieldNow, () => andThen(async((r) => r(succeed())), () => sync(() => log.push("finished")))),
        ),
      ),
      () => sync(() => log.push("unreachable")),
    ),
  );
  // Requested once the region has begun.
  runtime.fork(sync(() => runtime.interrupt(fiber)));
  flush();
  assert.deepEqual(log, ["finished"]);
  assert.deepEqual(fiber.exit, Exit.interrupt());
});

test("a host call gets an AbortSignal that interruption aborts", async () => {
  const runtime = new Runtime();
  let signal;
  let settle;
  const fiber = runtime.fork(
    fromPromise((s) => {
      signal = s;
      return new Promise((resolve) => (settle = resolve));
    }),
  );
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(signal.aborted, false);
  const exit = await runtime.interrupt(fiber);
  assert.deepEqual(exit, Exit.interrupt());
  assert.equal(signal.aborted, true);
  assert.ok(signal.reason instanceof Interrupted);
  settle("too late");
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(fiber.exit, Exit.interrupt());
});

test("a host call's promise decides its exit", async () => {
  const runtime = new Runtime({ reportDefect: () => {} });
  const ok = await runtime.run(fromPromise(async () => "value"));
  const rejected = await runtime.run(fromPromise(() => Promise.reject(new Error("no"))));
  const thrown = await runtime.run(
    fromPromise(() => {
      throw new Error("sync");
    }),
  );
  assert.deepEqual(ok, Exit.success("value"));
  assert.equal(rejected.$, "fail");
  assert.equal(rejected.error.message, "no");
  assert.equal(thrown.$, "defect");
  assert.equal(thrown.defect.message, "sync");
});
