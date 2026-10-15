import assert from "node:assert/strict";
import { test } from "node:test";

import { manual, shuffled } from "../fixtures/driver.js";
import { check, rng as rngFor } from "../src/check.js";
import {
  andThen,
  async,
  Exit,
  interrupt,
  interruptFork,
  map,
  receive,
  Runtime,
  succeed,
  sync,
  uninterruptible,
  yieldNow,
} from "../src/fiber.js";

// ---------------------------------------------------------------------------
// Model: receivers each taking a number of messages, sent to them by the
// host and by other fibers in a random order, some interrupted midway.
// Whatever the schedule, a receiver takes its messages in the order they
// were sent, never one twice, and waits exactly when it wants more than it
// was sent.

const count = (xs, p) => xs.filter(p).length;

/** Waits for the host to resume it, through `pending`, once in a while. */
function maybeAsync(r, pending, task) {
  if (!r.bool(0.3)) return task;
  return andThen(
    async((resume) => pending.push(() => resume(succeed()))),
    () => task,
  );
}

/** A receiver's task: `want` messages, each logged, with yields and host waits between them. */
function receiver(r, pending, want, log) {
  const steps = Array.from({ length: want }, () => r.int(0, 2));
  const take = (i) =>
    i === want
      ? succeed(log.length)
      : andThen(
          receive((message) => sync(() => log.push(message))),
          () => (steps[i] === 1 ? andThen(yieldNow, () => take(i + 1)) : maybeAsync(r, pending, take(i + 1))),
        );
  return take(0);
}

test("messages arrive in the order they were sent, once each, under any schedule", async () => {
  await check(
    (r) => ({
      receivers: r.array(() => ({ want: r.int(0, 4), sent: r.int(0, 5), killed: r.bool(0.2) }), 1, 3),
      schedule: r.next(),
    }),
    ({ receivers, schedule }) => {
      const r = rngFor(schedule);
      const { runtime, pending, settle } = shuffled(r, { synchronous: r.bool(0.5) });
      const sent = receivers.map(() => []);
      const logs = receivers.map(() => []);
      const send = (i, message) => {
        sent[i].push(message);
        runtime.send(fibers[i], message);
      };
      const fibers = receivers.map(({ want }, i) => runtime.fork(receiver(r, pending, want, logs[i])));
      receivers.forEach(({ sent: total, killed }, i) => {
        for (let k = 0; k < total; k++) {
          const message = `${i}.${k}`;
          if (r.bool(0.5)) pending.push(() => send(i, message));
          else runtime.fork(maybeAsync(r, pending, sync(() => send(i, message))));
        }
        if (killed) pending.push(() => runtime.interrupt(fibers[i]));
      });
      settle();

      receivers.forEach(({ want, sent: total, killed }, i) => {
        const fiber = fibers[i];
        assert.deepEqual(logs[i], sent[i].slice(0, logs[i].length), `receiver ${i} took ${logs[i]}`);
        if (fiber.exit?.$ === "interrupt") {
          assert.ok(killed);
          assert.equal(fiber.mailbox, null);
        } else if (total >= want) {
          assert.deepEqual(fiber.exit, Exit.success(want));
          assert.equal(fiber.mailbox, null);
        } else {
          // Waiting for a message that never comes.
          assert.equal(fiber.exit, null);
          assert.equal(fiber.status, "suspended");
          assert.deepEqual(logs[i], sent[i]);
        }
      });
      assert.equal(runtime.stats.finished, runtime.stats.started - count(fibers, (f) => f.exit === null));
    },
  );
});

// ---------------------------------------------------------------------------
// Unit tests

test("receive takes the oldest message, and waits while there is none", () => {
  const { runtime, flush } = manual();
  const log = [];
  const take = () => receive((message) => sync(() => log.push(message)));
  const fiber = runtime.fork(andThen(take(), () => andThen(take(), () => take())));
  runtime.send(fiber, "a");
  runtime.send(fiber, "b");
  flush();
  assert.deepEqual(log, ["a", "b"]);
  assert.equal(fiber.status, "suspended");
  runtime.send(fiber, "c");
  flush();
  assert.deepEqual(log, ["a", "b", "c"]);
  assert.deepEqual(fiber.exit, Exit.success(3));
});

test("a fiber that is done drops what is sent to it", () => {
  const { runtime, flush } = manual();
  const fiber = runtime.fork(succeed("done"));
  flush();
  runtime.send(fiber, "late");
  assert.equal(fiber.mailbox, null);
});

test("an interrupted receiver stops waiting, and its mailbox goes with it", () => {
  const { runtime, flush } = manual();
  const fiber = runtime.fork(andThen(receive(succeed), () => receive(succeed)));
  runtime.send(fiber, 1);
  flush();
  runtime.interrupt(fiber);
  flush();
  assert.deepEqual(fiber.exit, Exit.interrupt());
  runtime.send(fiber, 2);
  assert.equal(fiber.mailbox, null);
});

test("a receive that throws is a defect of its fiber", () => {
  const { runtime, flush, reports } = manual();
  const boom = new Error("boom");
  const fiber = runtime.fork(
    receive(() => {
      throw boom;
    }),
  );
  runtime.send(fiber, 1);
  flush();
  assert.deepEqual(fiber.exit, Exit.defect(boom));
  assert.deepEqual(reports, [boom]);
});

test("a synchronous runtime runs what the host starts or sends before the call returns", () => {
  const { runtime, slices } = manual({ synchronous: true });
  const log = [];
  const fiber = runtime.fork(
    andThen(sync(() => log.push("started")), () =>
      receive((message) => andThen(sync(() => log.push(message)), () => receive(succeed))),
    ),
  );
  assert.deepEqual(log, ["started"]);
  runtime.send(fiber, "ping");
  assert.deepEqual(log, ["started", "ping"]);
  // Work started inside a drain joins it rather than draining again.
  runtime.fork(sync(() => runtime.send(fiber, "pong")));
  assert.deepEqual(fiber.exit, Exit.success("pong"));
  assert.equal(slices.length, 0);
});

test("a synchronous runtime leaves what is past its budget to the host", () => {
  const { runtime, slices, flush } = manual({ synchronous: true, budget: 100, yieldEvery: 10 });
  let n = 0;
  const spin = () => andThen(sync(() => n++), () => (n === 1000 ? succeed(n) : spin()));
  const fiber = runtime.fork(spin());
  assert.ok(n > 0 && n < 1000, `${n} steps ran at once`);
  assert.equal(slices.length, 1);
  flush();
  assert.deepEqual(fiber.exit, Exit.success(1000));
});

test("interruptFork asks without waiting; interrupt waits", () => {
  const { runtime, flush } = manual();
  let resume;
  // Uninterruptible, so it ends only once the host resumes it.
  const busy = () => uninterruptible(async((r) => (resume = r)));
  const target = runtime.fork(busy());
  const asker = runtime.fork(map(interruptFork(target), () => "asked"));
  flush();
  assert.deepEqual(asker.exit, Exit.success("asked"));
  assert.equal(target.exit, null);
  const other = runtime.fork(busy());
  const waiter = runtime.fork(interrupt(other));
  flush();
  assert.equal(waiter.exit, null);
  resume(succeed("finished"));
  flush();
  assert.deepEqual(other.exit, Exit.interrupt());
  assert.deepEqual(waiter.exit, Exit.success(Exit.interrupt()));
});

test("interruptFork of yourself stops you at once, or when your region ends", () => {
  const { runtime, flush } = manual();
  const log = [];
  const stop = (me) => andThen(interruptFork(me()), () => sync(() => log.push("went on")));
  const plain = runtime.fork(andThen(yieldNow, () => stop(() => plain)));
  const masked = runtime.fork(andThen(yieldNow, () => uninterruptible(stop(() => masked))));
  flush();
  assert.deepEqual(plain.exit, Exit.interrupt());
  assert.deepEqual(masked.exit, Exit.interrupt());
  assert.deepEqual(log, ["went on"]);
});
