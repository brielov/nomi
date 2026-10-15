// A worker for the lifecycle tests, picked by argv[2]: `worker`'s
// lifecycle on a runtime, then fibers started on it, as a `Platform.worker`
// entry does.
// It prints "ready" once it is waiting, so the test knows when to signal it.

import { worker } from "../src/main.js";
import { acquireRelease, andThen, receive, Runtime, scoped, sleep, sync } from "../src/fiber.js";

const say = (line) => sync(() => console.log(line));
const ticking = andThen(sleep(5), () => ticking);
const waiting = receive(() => waiting);

const scenarios = {
  // A timer is live until signalled, and the ticker's release runs then.
  ticking: () =>
    scoped(
      andThen(
        acquireRelease(say("ticker acquired"), () => say("ticker released")),
        () => andThen(say("ready"), () => ticking),
      ),
    ),
  // A fiber waiting on its mailbox, as an effect manager does, holds no
  // timer: the process ends on its own.
  idle: () => andThen(say("ready"), () => waiting),
};

const runtime = new Runtime({ synchronous: true });
worker(runtime);
runtime.fork(scenarios[process.argv[2]]());
