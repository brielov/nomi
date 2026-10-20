// A program for the lifecycle tests, picked by argv[2], on the synchronous
// runtime programs run on. It prints "ready" once it is waiting, so the
// test knows when to signal it.

import { allowShutdown, main, requestExit } from "../src/main.js";
import {
  acquireRelease,
  andThen,
  async,
  die,
  fail,
  forkDaemon,
  join,
  Runtime,
  scoped,
  sleep,
  succeed,
  sync,
  wait,
  yieldNow,
} from "../src/fiber.js";

const runtime = new Runtime({ synchronous: true });
const say = (line) => sync(() => console.log(line));
const forever = async(() => {
  const handle = setInterval(() => {}, 1 << 30);
  return () => clearInterval(handle);
});
const resource = (name, release = say(`${name} released`)) =>
  acquireRelease(say(`${name} acquired`), () => release);
const ready = (rest) => andThen(say("ready"), () => rest);

const slowRelease = (allow) =>
  andThen(sync(() => allowShutdown(runtime, allow)), () =>
    scoped(andThen(resource("slow", andThen(sleep(400), () => say("slow released"))), () => ready(forever))),
  );

const scenarios = {
  // Waits until signalled; both resources are released, last first.
  wait: () => scoped(andThen(resource("a"), () => andThen(resource("b"), () => ready(forever)))),
  // A finalizer that never ends: the deadline cuts it short.
  hang: () => scoped(andThen(resource("stuck", andThen(say("releasing"), () => forever)), () => ready(forever))),
  // A finalizer slower than the deadline, which the program lengthened.
  allowed: () => slowRelease(2_000),
  // As `allowed`, by more than a host timer can wait.
  allowedLong: () => slowRelease(2 ** 32),
  // Ends on its own; the daemon it left is interrupted and released.
  daemon: () =>
    andThen(forkDaemon(scoped(andThen(resource("daemon"), () => forever))), () =>
      // Let the daemon start first.
      andThen(yieldNow, () => andThen(say("main done"), () => succeed())),
    ),
  // Ends on its own, leaving a daemon that waits on another: interrupting
  // the first ends the second before the shutdown reaches it.
  dependent: () => andThen(forkDaemon(forever), (first) => andThen(forkDaemon(join(first)), () => say("main done"))),
  // A finalizer that dies while a signal stops the program.
  broken: () => scoped(andThen(resource("a", die(new Error("release broke"))), () => ready(forever))),
  // Asks to exit 3 and waits; its resource is released first.
  exit: () => scoped(andThen(resource("a"), () => andThen(sync(() => requestExit(runtime, 3)), () => forever))),
  // Waits for another fiber that never wakes it, and nothing else is
  // left to happen: it is stuck, and its resource is released.
  stuck: () => scoped(andThen(resource("a"), () => wait(() => {}))),
  ok: () => say("hello"),
  fail: () => fail("bad input"),
  defect: () => die(new Error("broken")),
};

main(scenarios[process.argv[2]](), { runtime, deadline: Number(process.argv[3] ?? 10_000) });
