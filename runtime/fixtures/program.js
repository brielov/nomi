// A program for the lifecycle tests, picked by argv[2]. It prints "ready"
// once it is waiting, so the test knows when to signal it.

import { allowShutdown, main } from "../src/main.js";
import {
  acquireRelease,
  andThen,
  async,
  die,
  fail,
  forkDaemon,
  Runtime,
  scoped,
  sleep,
  succeed,
  sync,
  yieldNow,
} from "../src/fiber.js";

const runtime = new Runtime();
const say = (line) => sync(() => console.log(line));
const forever = async(() => {
  const handle = setInterval(() => {}, 1 << 30);
  return () => clearInterval(handle);
});
const resource = (name, release = say(`${name} released`)) =>
  acquireRelease(say(`${name} acquired`), () => release);
const ready = (rest) => andThen(say("ready"), () => rest);

const scenarios = {
  // Waits until signalled; both resources are released, last first.
  wait: () => scoped(andThen(resource("a"), () => andThen(resource("b"), () => ready(forever)))),
  // A finalizer that never ends: the deadline cuts it short.
  hang: () => scoped(andThen(resource("stuck", andThen(say("releasing"), () => forever)), () => ready(forever))),
  // A finalizer slower than the deadline, which the program lengthened.
  allowed: () =>
    andThen(sync(() => allowShutdown(runtime, 2_000)), () =>
      scoped(andThen(resource("slow", andThen(sleep(400), () => say("slow released"))), () => ready(forever))),
    ),
  // Ends on its own; the daemon it left is interrupted and released.
  daemon: () =>
    andThen(forkDaemon(scoped(andThen(resource("daemon"), () => forever))), () =>
      // Let the daemon start first.
      andThen(yieldNow, () => andThen(say("main done"), () => succeed())),
    ),
  ok: () => say("hello"),
  fail: () => fail("bad input"),
  defect: () => die(new Error("broken")),
};

main(scenarios[process.argv[2]](), { runtime, deadline: Number(process.argv[3] ?? 10_000) });
